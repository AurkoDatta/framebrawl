//! The real server, reached over its WebSocket transport by real clients.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use fb_client::{Client, Event};
use fb_host::RoomOptions;
use fb_replay::Replay;
use fb_rollback::SessionConfig;
use fb_server::{Server, ServerConfig};
use fb_sim::{Input, MatchConfig};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

async fn start(room: RoomOptions, data_dir: Option<PathBuf>) -> SocketAddr {
    let server = Server::bind(ServerConfig {
        http: "127.0.0.1:0".parse().expect("address"),
        udp: None,
        public_ips: Vec::new(),
        static_dir: None,
        data_dir,
        room,
        silence_limit: Duration::from_millis(500),
    })
    .await
    .expect("server binds");
    let address = server.http;
    tokio::spawn(server.run());
    address
}

/// Plays over the WebSocket for `ticks` 60 Hz ticks and returns the client
/// and everything it reported.
async fn play(address: SocketAddr, room: &str, ticks: u32) -> (Client, Vec<Event>) {
    let url = format!("ws://{address}/api/ws?room={room}");
    let (socket, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("connects");
    let (mut sink, mut stream) = socket.split();
    let mut client = Client::new(SessionConfig::default());
    let mut events = Vec::new();
    let started = tokio::time::Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_nanos(1_000_000_000 / 60));
    let mut tick = 0;

    while tick < ticks {
        let now_ms = started.elapsed().as_millis() as u32;
        tokio::select! {
            message = stream.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    client.receive(&bytes, now_ms).expect("valid server message");
                }
                Some(Ok(_)) => {}
                other => panic!("socket ended: {other:?}"),
            },
            _ = ticker.tick() => {
                tick += 1;
                // Run in a small circle, shoving now and then.
                let buttons = Input::MOVE | if tick % 40 == 0 { Input::SHOVE } else { 0 };
                let input = Input::new(buttons, (tick * 2) as u8);
                for message in client.tick(input, now_ms) {
                    sink.send(Message::Binary(message.bytes.into())).await.expect("sends");
                }
            }
        }
        while let Some(event) = client.next_event() {
            events.push(event);
        }
    }
    (client, events)
}

async fn http_get(address: SocketAddr, path: &str) -> (u16, Vec<u8>) {
    let mut stream = TcpStream::connect(address).await.expect("connects");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("writes");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("reads");
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header end");
    let head = String::from_utf8_lossy(&response[..split]).to_string();
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status");
    let mut body = response[split + 4..].to_vec();
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        body = dechunk(&body);
    }
    (status, body)
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = data.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&data[..line_end]).trim(), 16)
            .expect("chunk size");
        if size == 0 {
            break;
        }
        let start = line_end + 2;
        out.extend_from_slice(&data[start..start + size]);
        data = &data[start + size + 2..];
    }
    out
}

fn no_desync(events: &[Event]) -> bool {
    !events.iter().any(|e| matches!(e, Event::Desync { .. }))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_plays_against_bots_in_real_time() {
    let address = start(RoomOptions::default(), None).await;
    let (client, events) = play(address, "solo", 240).await;

    assert!(
        matches!(events[0], Event::Welcome { player: 0, .. }),
        "{:?}",
        events[0]
    );
    assert!(
        matches!(events[1], Event::Synced { frame } if frame < 30),
        "{:?}",
        events[1]
    );
    assert!(no_desync(&events));
    let stats = client.stats();
    assert!(
        client.confirmed_frame() > 200,
        "confirmed {}",
        client.confirmed_frame()
    );
    assert!(stats.checkpoints_verified >= 5, "{stats:?}");
    assert!(stats.rollbacks > 0, "bots change direction: {stats:?}");
    assert!(
        client.rtt_ms() < 60,
        "loopback round trip {} ms",
        client.rtt_ms()
    );
    let world = client.world().expect("synced");
    assert_eq!(world.player_count(), 4, "three bots fill the room");
    assert!(
        world.frame() >= client.confirmed_frame(),
        "predicting ahead of the server"
    );

    let (status, body) = http_get(address, "/api/health").await;
    assert_eq!(status, 200);
    let health = String::from_utf8(body).expect("utf8");
    assert!(
        health.contains(r#""status":"ok""#) && health.contains(r#""webrtc":false"#),
        "{health}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_clients_share_a_room_and_the_second_joins_late() {
    let address = start(RoomOptions::default(), None).await;
    let first = tokio::spawn(play(address, "pair", 300));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (second, events) = play(address, "pair", 180).await;
    let (first, first_events) = first.await.expect("first client");

    assert_eq!((first.player(), second.player()), (Some(0), Some(1)));
    let joined_at = events.iter().find_map(|e| match e {
        Event::Synced { frame } => Some(*frame),
        _ => None,
    });
    assert!(
        joined_at.is_some_and(|f| (60..=120).contains(&f)),
        "joined at {joined_at:?}"
    );
    assert!(no_desync(&events) && no_desync(&first_events));
    assert!(second.stats().checkpoints_verified >= 3);
    assert_eq!(first.config(), second.config());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_match_is_stored_and_served_as_a_valid_replay() {
    let dir = std::env::temp_dir().join(format!("framebrawl-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let room = RoomOptions {
        config: MatchConfig {
            duration_frames: 150,
            score_limit: u16::MAX,
            ..MatchConfig::default()
        },
        intermission_ticks: 20,
        ..RoomOptions::default()
    };
    let address = start(room, Some(dir.clone())).await;
    let (_, events) = play(address, "short", 260).await;

    let over = events.iter().find_map(|e| match e {
        Event::MatchOver { scores, .. } => Some(scores.clone()),
        _ => None,
    });
    let scores = over.expect("match ended");
    let welcomes = events
        .iter()
        .filter(|e| matches!(e, Event::Welcome { .. }))
        .count();
    assert_eq!(welcomes, 2, "the next match started");

    let (status, body) = http_get(address, "/api/replays").await;
    assert_eq!(status, 200);
    let listing = String::from_utf8(body).expect("utf8");
    let name = listing
        .split(r#""name":""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no replay listed: {listing}"));

    let (status, bytes) = http_get(address, &format!("/api/replays/{name}")).await;
    assert_eq!(status, 200);
    let replay = Replay::decode(&bytes).expect("a replay");
    assert_eq!(replay.frames(), 150);
    let world = replay.simulate().expect("verifies against its checkpoints");
    let replayed: Vec<u16> = (0..4).map(|p| world.player(p).score).collect();
    assert_eq!(replayed, scores);

    let (status, _) = http_get(address, "/api/replays/..%2F..%2Fetc%2Fpasswd").await;
    assert_eq!(status, 404);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_sends_garbage_is_dropped_and_the_room_survives() {
    let address = start(RoomOptions::default(), None).await;
    let url = format!("ws://{address}/api/ws?room=rude");
    let (mut socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connects");
    socket
        .send(Message::Binary(vec![0xff; 40].into()))
        .await
        .expect("sends");
    let closed = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "server closes the socket");

    let (client, events) = play(address, "rude", 90).await;
    assert!(no_desync(&events));
    assert!(client.confirmed_frame() > 50);

    let (status, _) = http_get(address, "/api/ws?room=not%20ok").await;
    assert_eq!(status, 400);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_goes_silent_loses_its_seat() {
    let address = start(RoomOptions::default(), None).await;
    let url = format!("ws://{address}/api/ws?room=quiet");
    let (mut socket, _) = tokio_tungstenite::connect_async(&url)
        .await
        .expect("connects");
    // Never send anything. The socket stays open from our side, so only
    // the room's own timeout can end this.
    let closed = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(Ok(message)) = socket.next().await {
            if message.is_close() {
                break;
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "server gives the seat up");
}
