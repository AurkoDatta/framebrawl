//! A server and its clients joined by a simulated bad network, driven one
//! 60 Hz tick at a time.
#![allow(dead_code)]

use std::collections::VecDeque;

use fb_rollback::{
    Authority, ClientPacket, Rollback, ServerPacket, Session, SessionConfig, Simulation,
};
use fb_sim::canon::Field;
use fb_sim::{Input, Pcg32};

/// A simulation small enough to run millions of frames in a test, where
/// any wrong or reordered input changes every later checksum.
#[derive(Clone, Debug, PartialEq)]
pub struct ToySim {
    pub frame: u32,
    pub cells: Vec<u64>,
    /// Corrupts the state on this frame, to stage a desync.
    pub glitch_at: Option<u32>,
}

impl ToySim {
    pub fn new(players: usize) -> Self {
        ToySim {
            frame: 0,
            cells: vec![0x9e37_79b9; players],
            glitch_at: None,
        }
    }
}

impl Simulation for ToySim {
    fn step(&mut self, inputs: &[Input]) {
        let mut carry = self.frame as u64;
        for (cell, input) in self.cells.iter_mut().zip(inputs) {
            let word = (input.buttons as u64) << 8 | input.aim as u64;
            *cell = cell
                .wrapping_mul(0x0000_0100_0000_01b3)
                .wrapping_add(word ^ carry);
            carry = *cell >> 7;
        }
        if self.glitch_at == Some(self.frame) {
            self.cells[0] ^= 1;
        }
        self.frame += 1;
    }

    fn checksum(&self) -> u64 {
        self.cells
            .iter()
            .fold(self.frame as u64, |h, c| h.rotate_left(9) ^ c)
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field {
            name: "frame",
            index: 0,
            bits: self.frame as u64,
        }];
        fields.extend(self.cells.iter().enumerate().map(|(i, &bits)| Field {
            name: "cell",
            index: i as u32,
            bits,
        }));
        fields
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LinkQuality {
    /// One-way delay in ticks.
    pub delay: u32,
    /// Extra random delay, 0 to this many ticks. Causes reordering.
    pub jitter: u32,
    pub loss_percent: u32,
    pub duplicate_percent: u32,
}

impl LinkQuality {
    pub const PERFECT: LinkQuality = LinkQuality {
        delay: 0,
        jitter: 0,
        loss_percent: 0,
        duplicate_percent: 0,
    };

    /// 150 ms round trip with 5% loss: the design target. A packet takes
    /// one tick to send plus `delay` plus zero or one of jitter, so 4 to 5
    /// ticks each way.
    pub const TARGET: LinkQuality = LinkQuality {
        delay: 3,
        jitter: 1,
        loss_percent: 5,
        duplicate_percent: 0,
    };
}

struct Link<T> {
    in_flight: VecDeque<(u64, T)>,
}

impl<T: Clone> Link<T> {
    fn new() -> Self {
        Link {
            in_flight: VecDeque::new(),
        }
    }

    fn send(&mut self, now: u64, packet: T, quality: LinkQuality, rng: &mut Pcg32) {
        if rng.below(100) < quality.loss_percent {
            return;
        }
        let copies = 1 + (rng.below(100) < quality.duplicate_percent) as u32;
        for _ in 0..copies {
            let extra = if quality.jitter > 0 {
                rng.below(quality.jitter + 1)
            } else {
                0
            };
            let due = now + 1 + (quality.delay + extra) as u64;
            self.in_flight.push_back((due, packet.clone()));
        }
    }

    fn deliver(&mut self, now: u64) -> Vec<T> {
        let mut due = Vec::new();
        self.in_flight.retain(|(at, packet)| {
            let arrived = *at <= now;
            if arrived {
                due.push(packet.clone());
            }
            !arrived
        });
        due
    }
}

/// A player mashing buttons: holds each choice for a random stretch.
struct Masher {
    rng: Pcg32,
    held: Input,
    frames_left: u32,
}

impl Masher {
    fn next(&mut self) -> Input {
        if self.frames_left == 0 {
            self.held = Input::new(self.rng.below(16) as u8, self.rng.below(256) as u8);
            self.frames_left = 1 + self.rng.below(20);
        }
        self.frames_left -= 1;
        self.held
    }
}

pub struct Client<S> {
    pub session: Session<S>,
    pub rollbacks: Vec<Rollback>,
    /// Largest number of frames predicted past the confirmed frame.
    pub deepest_prediction: u32,
    masher: Masher,
    up: Link<ClientPacket>,
    down: Link<ServerPacket>,
}

pub struct Net<S> {
    pub authority: Authority<S>,
    pub clients: Vec<Client<S>>,
    pub quality: LinkQuality,
    pub tick: u64,
    rng: Pcg32,
}

impl<S: Simulation> Net<S> {
    pub fn new(
        sim: S,
        players: usize,
        config: SessionConfig,
        quality: LinkQuality,
        seed: u64,
    ) -> Self {
        let clients = (0..players)
            .map(|player| Client {
                session: Session::new(sim.clone(), players, player, config),
                rollbacks: Vec::new(),
                deepest_prediction: 0,
                masher: Masher {
                    rng: Pcg32::new(seed.wrapping_mul(31).wrapping_add(player as u64)),
                    held: Input::default(),
                    frames_left: 0,
                },
                up: Link::new(),
                down: Link::new(),
            })
            .collect();
        Net {
            authority: Authority::new(sim, players, config.checkpoint_interval),
            clients,
            quality,
            tick: 0,
            rng: Pcg32::new(seed),
        }
    }

    pub fn run(&mut self, ticks: u32) {
        for _ in 0..ticks {
            self.step();
        }
    }

    pub fn step(&mut self) {
        let now = self.tick;
        for (player, client) in self.clients.iter_mut().enumerate() {
            for packet in client.up.deliver(now) {
                self.authority.receive(player, &packet);
            }
        }
        self.authority.tick();
        for (player, client) in self.clients.iter_mut().enumerate() {
            match self.authority.packet_for(player) {
                Some(packet) => client.down.send(now, packet, self.quality, &mut self.rng),
                None => {
                    // Too far behind for input history: hand over a snapshot,
                    // as the server does over its reliable channel.
                    client.session.resync(
                        self.authority.frame(),
                        self.authority.state().clone(),
                        self.authority.last_inputs(),
                    );
                    self.authority.snapshot_sent(player);
                }
            }
        }

        for client in &mut self.clients {
            for packet in client.down.deliver(now) {
                client.rollbacks.extend(client.session.receive(&packet));
            }
            for _ in 0..client.session.frames_due() {
                client.session.advance(client.masher.next());
            }
            let ahead = client.session.frame() - client.session.confirmed_frame();
            client.deepest_prediction = client.deepest_prediction.max(ahead);
            client
                .up
                .send(now, client.session.outgoing(), self.quality, &mut self.rng);
        }
        self.tick += 1;
    }

    /// Lets everything in flight arrive over a clean link.
    pub fn settle(&mut self) {
        self.quality = LinkQuality::PERFECT;
        self.run(120);
    }
}
