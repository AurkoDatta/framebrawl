//! A room and real clients talking in encoded bytes over impaired links.

use fb_client::{Client, Event};
use fb_host::{FinishedMatch, Outgoing, Room, RoomOptions};
use fb_net::{ClientMessage, ImpairedLink, Impairment};
use fb_rollback::SessionConfig;
use fb_sim::{Input, MatchConfig, Pcg32};

const TICK_US: u64 = 1_000_000 / 60;

struct Player {
    seat: usize,
    client: Client,
    events: Vec<Event>,
    /// Unreliable traffic takes the impaired link; reliable traffic is
    /// delayed but never lost or reordered, as on a real reliable channel.
    up: ImpairedLink<Vec<u8>>,
    down: ImpairedLink<Vec<u8>>,
    up_reliable: ImpairedLink<Vec<u8>>,
    down_reliable: ImpairedLink<Vec<u8>>,
    rng: Pcg32,
    held: Input,
}

struct Table {
    room: Room,
    players: Vec<Player>,
    impairment: Impairment,
    now_us: u64,
    finished: Vec<FinishedMatch>,
}

impl Table {
    fn new(options: RoomOptions, impairment: Impairment) -> Self {
        Table {
            room: Room::new(99, options),
            players: Vec::new(),
            impairment,
            now_us: 0,
            finished: Vec::new(),
        }
    }

    fn join(&mut self) -> usize {
        let mut out = Vec::new();
        let seat = self.room.join(&mut out).expect("a free seat");
        let reliable = Impairment {
            latency_ms: self.impairment.latency_ms,
            ..Impairment::default()
        };
        let seed = 1000 + self.players.len() as u64 * 10;
        self.players.push(Player {
            seat,
            client: Client::new(SessionConfig::default()),
            events: Vec::new(),
            up: ImpairedLink::new(self.impairment, seed),
            down: ImpairedLink::new(self.impairment, seed + 1),
            up_reliable: ImpairedLink::new(reliable, seed + 2),
            down_reliable: ImpairedLink::new(reliable, seed + 3),
            rng: Pcg32::new(seed + 4),
            held: Input::default(),
        });
        self.route(out);
        self.players.len() - 1
    }

    fn route(&mut self, out: Vec<Outgoing>) {
        let players = self.room.players();
        for message in out {
            let Some(player) = self.players.iter_mut().find(|p| p.seat == message.player) else {
                continue;
            };
            let bytes = message.message.encode(players);
            if message.reliable {
                player.down_reliable.send(self.now_us, bytes);
            } else {
                player.down.send(self.now_us, bytes);
            }
        }
    }

    fn tick(&mut self) {
        let now_ms = (self.now_us / 1000) as u32;
        let mut out = Vec::new();
        for player in &mut self.players {
            let mut arrived = player.up_reliable.receive(self.now_us);
            arrived.extend(player.up.receive(self.now_us));
            for bytes in arrived {
                let message = ClientMessage::decode(&bytes).expect("client sends valid messages");
                self.room.receive(player.seat, message, &mut out);
            }
        }
        self.finished.extend(self.room.tick(&mut out));
        self.route(out);

        for player in &mut self.players {
            let mut arrived = player.down_reliable.receive(self.now_us);
            arrived.extend(player.down.receive(self.now_us));
            for bytes in arrived {
                player
                    .client
                    .receive(&bytes, now_ms)
                    .expect("server sends valid messages");
            }
            if player.rng.below(12) == 0 {
                player.held = Input::new(player.rng.below(16) as u8, player.rng.below(256) as u8);
            }
            for message in player.client.tick(player.held, now_ms) {
                if message.reliable {
                    player.up_reliable.send(self.now_us, message.bytes);
                } else {
                    player.up.send(self.now_us, message.bytes);
                }
            }
            while let Some(event) = player.client.next_event() {
                player.events.push(event);
            }
        }
        self.now_us += TICK_US;
    }

    fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.tick();
        }
    }

    /// Clears the link and lets everything arrive, then checks the client
    /// agrees with the room about the present.
    fn assert_in_sync(&mut self, index: usize) {
        for player in &mut self.players {
            player.up.set_impairment(Impairment::default());
            player.down.set_impairment(Impairment::default());
        }
        self.run(90);
        let player = &self.players[index];
        let desyncs = player
            .events
            .iter()
            .filter(|e| matches!(e, Event::Desync { .. }))
            .count();
        assert_eq!(desyncs, 0, "client {index} desynced");
        let confirmed = player.client.confirmed_frame();
        let server = self.room.world().frame();
        assert!(
            server - confirmed <= 2,
            "confirmed {confirmed}, server at {server}"
        );
        assert!(player.client.stats().checkpoints_verified > 0);
    }
}

fn long_match() -> RoomOptions {
    RoomOptions {
        config: MatchConfig {
            duration_frames: u32::MAX,
            score_limit: u16::MAX,
            ..MatchConfig::default()
        },
        ..RoomOptions::default()
    }
}

const TARGET: Impairment = Impairment {
    latency_ms: 65,
    jitter_ms: 20,
    loss_permille: 50,
    duplicate_permille: 0,
};

#[test]
fn one_human_and_three_bots_stay_in_sync_over_a_bad_link() {
    let mut table = Table::new(long_match(), TARGET);
    let me = table.join();
    table.run(60 * 60);
    let client = &table.players[me].client;
    assert!(client.stats().rollbacks > 200, "{:?}", client.stats());
    assert_eq!(client.stats().stalls, 0);
    let rtt = client.rtt_ms();
    assert!((130..=200).contains(&rtt), "measured round trip {rtt} ms");
    table.assert_in_sync(me);
}

#[test]
fn joining_mid_match_replays_history_and_takes_over_a_bot() {
    let mut table = Table::new(long_match(), TARGET);
    let first = table.join();
    table.run(1500);
    let late = table.join();
    assert_eq!(table.room.humans(), 2);
    table.run(600);

    let events = &table.players[late].events;
    assert!(
        matches!(events[0], Event::Welcome { player: 1, .. }),
        "{:?}",
        events[0]
    );
    let Event::Synced { frame } = events[1] else {
        panic!("expected sync, got {:?}", events[1]);
    };
    assert_eq!(frame, 1500, "joined at the frame the room was on");
    table.assert_in_sync(late);
    table.assert_in_sync(first);
}

#[test]
fn a_seat_left_empty_goes_back_to_a_bot_and_can_be_retaken() {
    let mut table = Table::new(long_match(), Impairment::default());
    let me = table.join();
    table.run(300);
    let seat = table.players[me].seat;
    table.room.leave(seat);
    table.players.clear();
    assert_eq!(table.room.humans(), 0);
    table.run(300);
    assert_eq!(table.room.world().frame(), 600, "the match carries on");

    let again = table.join();
    assert_eq!(table.players[again].seat, seat);
    table.run(300);
    table.assert_in_sync(again);
}

#[test]
fn a_finished_match_yields_a_verifiable_replay_and_a_new_match_follows() {
    let options = RoomOptions {
        config: MatchConfig {
            duration_frames: 400,
            score_limit: u16::MAX,
            ..MatchConfig::default()
        },
        intermission_ticks: 30,
        ..RoomOptions::default()
    };
    let mut table = Table::new(options, TARGET);
    let me = table.join();
    table.run(400 + 30 + 300);

    assert_eq!(table.finished.len(), 1);
    let finished = &table.finished[0];
    assert_eq!(finished.replay.frames(), 400);
    let replayed = finished.replay.simulate().expect("replay verifies");
    let scores: Vec<u16> = (0..4).map(|p| replayed.player(p).score).collect();
    assert_eq!(scores, finished.scores);

    let events = &table.players[me].events;
    let over = events
        .iter()
        .position(|e| matches!(e, Event::MatchOver { .. }));
    let welcomes: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Welcome { config, .. } => Some(config.seed),
            _ => None,
        })
        .collect();
    assert!(over.is_some(), "client was told the match ended");
    assert_eq!(welcomes.len(), 2, "and welcomed into the next one");
    assert_ne!(welcomes[0], welcomes[1], "with a fresh seed");
    assert!(
        table.room.world().frame() > 200,
        "second match is under way"
    );
    table.assert_in_sync(me);
}

#[test]
fn four_humans_fill_the_room_and_a_fifth_is_turned_away() {
    let mut table = Table::new(long_match(), TARGET);
    for _ in 0..4 {
        table.join();
        table.run(120);
    }
    let mut out = Vec::new();
    assert_eq!(table.room.join(&mut out), None);
    table.run(1200);
    for index in 0..4 {
        table.assert_in_sync(index);
    }
}

#[test]
fn a_blackout_past_the_input_history_resyncs_by_replay() {
    let mut table = Table::new(long_match(), Impairment::default());
    let me = table.join();
    table.run(300);
    let blackout = Impairment {
        loss_permille: 1000,
        ..Impairment::default()
    };
    table.players[me].up.set_impairment(blackout);
    table.players[me].down.set_impairment(blackout);
    table.run(400);
    let syncs = |table: &Table| {
        table.players[me]
            .events
            .iter()
            .filter(|e| matches!(e, Event::Synced { .. }))
            .count()
    };
    assert_eq!(
        syncs(&table),
        2,
        "history was resent over the reliable channel"
    );
    table.assert_in_sync(me);
}
