//! The Netcode Lab: a whole match in one object.
//!
//! A room and one client run side by side, joined by the same simulated
//! link the tests use. The page moves the sliders; this is the real
//! protocol code on both ends, with nothing mocked but the wire.

use fb_client::{Client, Event};
use fb_host::{Outgoing, Room, RoomOptions};
use fb_net::{ClientMessage, ImpairedLink, Impairment};
use fb_rollback::bisect::{first_divergent_field, first_divergent_frame};
use fb_rollback::{InputDelay, SessionConfig};
use fb_sim::Input;
use wasm_bindgen::prelude::*;

use crate::view::{encode_event, write_view};

const TICK_US: u64 = 1_000_000 / 60;

/// One direction of the connection: a lossy path for per-tick messages
/// and a delayed but dependable one for the rest, like the two channels
/// of the real transport.
struct Path {
    lossy: ImpairedLink<Vec<u8>>,
    reliable: ImpairedLink<Vec<u8>>,
    bytes: u64,
}

impl Path {
    fn new(seed: u64) -> Self {
        Path {
            lossy: ImpairedLink::new(Impairment::default(), seed),
            reliable: ImpairedLink::new(Impairment::default(), seed + 1),
            bytes: 0,
        }
    }

    fn set(&mut self, impairment: Impairment) {
        self.lossy.set_impairment(impairment);
        self.reliable.set_impairment(Impairment {
            latency_ms: impairment.latency_ms,
            ..Impairment::default()
        });
    }

    fn send(&mut self, now_us: u64, reliable: bool, bytes: Vec<u8>) {
        self.bytes += bytes.len() as u64;
        if reliable {
            self.reliable.send(now_us, bytes);
        } else {
            self.lossy.send(now_us, bytes);
        }
    }

    fn receive(&mut self, now_us: u64) -> Vec<Vec<u8>> {
        let mut arrived = self.reliable.receive(now_us);
        arrived.extend(self.lossy.receive(now_us));
        arrived
    }
}

#[wasm_bindgen]
pub struct Lab {
    room: Room,
    client: Client,
    seat: usize,
    up: Path,
    down: Path,
    now_us: u64,
    events: Vec<Vec<f64>>,
    /// Deepest rollback during the latest tick.
    rollback_depth: u32,
    desyncs: u32,
    report: String,
    view: Vec<f32>,
}

#[wasm_bindgen]
impl Lab {
    /// `input_delay` is a fixed number of frames, or negative to let the
    /// client choose.
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u64, input_delay: i32) -> Lab {
        let mut room = Room::new(seed, RoomOptions::default());
        let session = SessionConfig {
            input_delay: match u8::try_from(input_delay) {
                Ok(frames) => InputDelay::Fixed(frames.min(8)),
                Err(_) => InputDelay::Adaptive { max: 3 },
            },
            ..SessionConfig::default()
        };
        let mut out = Vec::new();
        let seat = room.join(&mut out).expect("a new room has a free seat");
        let mut lab = Lab {
            room,
            client: Client::new(session),
            seat,
            up: Path::new(seed ^ 0x75),
            down: Path::new(seed ^ 0xd0),
            now_us: 0,
            events: Vec::new(),
            rollback_depth: 0,
            desyncs: 0,
            report: String::new(),
            view: Vec::new(),
        };
        lab.route(out);
        lab
    }

    /// Sets both directions of the link. `latency_ms` is one way.
    pub fn set_network(
        &mut self,
        latency_ms: u32,
        jitter_ms: u32,
        loss_permille: u32,
        duplicate_permille: u32,
    ) {
        let impairment = Impairment {
            latency_ms: latency_ms.min(1000),
            jitter_ms: jitter_ms.min(500),
            loss_permille: loss_permille.min(900),
            duplicate_permille: duplicate_permille.min(1000),
        };
        self.up.set(impairment);
        self.down.set(impairment);
    }

    /// One sixtieth of a second for everyone: the room, the wire and the
    /// client.
    pub fn tick(&mut self, buttons: u8, aim: u8) {
        self.now_us += TICK_US;
        let now_ms = (self.now_us / 1000) as u32;
        self.rollback_depth = 0;

        let mut out = Vec::new();
        for bytes in self.up.receive(self.now_us) {
            if let Ok(message) = ClientMessage::decode(&bytes) {
                self.room.receive(self.seat, message, &mut out);
            }
        }
        // The Lab keeps no replays, so a finished match is simply dropped.
        let _ = self.room.tick(&mut out);
        self.route(out);

        for bytes in self.down.receive(self.now_us) {
            // Both ends are this build, so a message that fails to decode
            // would be a bug in the codec; the next tick's packet covers it.
            let _ = self.client.receive(&bytes, now_ms);
            self.drain_events();
        }
        for message in self.client.tick(Input::new(buttons, aim), now_ms) {
            self.up.send(self.now_us, message.reliable, message.bytes);
        }
        self.drain_events();
    }

    /// Breaks the client's simulation on purpose.
    pub fn corrupt(&mut self) {
        self.client.corrupt();
    }

    /// What the last desync turned out to be, in words. Empty if there has
    /// not been one since the last call.
    pub fn take_report(&mut self) -> String {
        std::mem::take(&mut self.report)
    }

    pub fn next_event(&mut self) -> Vec<f64> {
        if self.events.is_empty() {
            Vec::new()
        } else {
            self.events.remove(0)
        }
    }

    /// The client's predicted world, in the same layout as `NetClient`.
    pub fn view(&mut self) -> Vec<f32> {
        write_view(&self.client, &mut self.view);
        self.view.clone()
    }

    /// Where the server has each player right now:
    /// `[frame, (x, y, z, facing, in play) per player]`.
    pub fn truth(&self) -> Vec<f32> {
        let world = self.room.world();
        let mut out = vec![world.frame() as f32];
        for index in 0..world.player_count() {
            let p = world.player(index);
            out.extend(p.position);
            out.extend([f32::from(p.facing), f32::from(u8::from(p.in_play))]);
        }
        out
    }

    /// The numbers the timeline plots for the latest tick:
    /// `[client frame, confirmed frame, server frame, rollback depth,
    /// input delay, round trip ms, stalls, late inputs, packets sent,
    /// packets lost, bytes up, bytes down, desyncs]`. Counts are totals.
    pub fn sample(&self) -> Vec<f64> {
        let stats = self.client.stats();
        let (sent_up, lost_up) = self.up.lossy.counts();
        let (sent_down, lost_down) = self.down.lossy.counts();
        vec![
            f64::from(self.client.frame()),
            f64::from(self.client.confirmed_frame()),
            f64::from(self.room.world().frame()),
            f64::from(self.rollback_depth),
            f64::from(self.client.input_delay()),
            f64::from(self.client.rtt_ms()),
            f64::from(stats.stalls),
            f64::from(stats.substituted_inputs),
            (sent_up + sent_down) as f64,
            (lost_up + lost_down) as f64,
            self.up.bytes as f64,
            self.down.bytes as f64,
            f64::from(self.desyncs),
        ]
    }
}

/// Small integers as they are; anything else is the bit pattern of a float
/// and is clearer in hex.
fn readable(bits: u64) -> String {
    if bits < 1 << 16 {
        bits.to_string()
    } else {
        format!("{bits:#x}")
    }
}

impl Lab {
    fn route(&mut self, out: Vec<Outgoing>) {
        let players = self.room.players();
        for message in out {
            if message.player == self.seat {
                let bytes = message.message.encode(players);
                self.down.send(self.now_us, message.reliable, bytes);
            }
        }
    }

    fn drain_events(&mut self) {
        while let Some(event) = self.client.next_event() {
            match &event {
                Event::Rollback(rollback) => {
                    self.rollback_depth = self.rollback_depth.max(rollback.depth);
                }
                Event::Desync { frame } => {
                    self.desyncs += 1;
                    self.report = self.explain_desync(*frame);
                }
                _ => {}
            }
            self.events.push(encode_event(event));
        }
    }

    /// The Lab can see both ends at once, so it can run the bisect on the
    /// spot instead of waiting for the server's answer to cross the link.
    fn explain_desync(&self, checkpoint: u32) -> String {
        let Some(session) = self.client.session() else {
            return String::new();
        };
        let authority = self.room.authority();
        let ours = session.recent_checksums();
        let theirs = authority.recent_checksums();
        let Some(frame) = first_divergent_frame(&ours, &theirs) else {
            return format!("Checkpoint {checkpoint} did not match the server.");
        };
        let field = session
            .dump(frame)
            .zip(authority.dump(frame))
            .and_then(|(ours, theirs)| first_divergent_field(&ours, &theirs));
        match field {
            Some(diff) => format!(
                "Checkpoint {checkpoint} did not match. First wrong frame: {frame}. \
                 First wrong field: {}[{}], client {}, server {}.",
                diff.name,
                diff.index,
                readable(diff.ours),
                readable(diff.theirs)
            ),
            None => format!("Checkpoint {checkpoint} did not match. First wrong frame: {frame}."),
        }
    }
}
