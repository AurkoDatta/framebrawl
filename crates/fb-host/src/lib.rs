//! A room: one running match and everything the server does for it.
//!
//! The room owns the authority, plays the bots, records the replay and
//! speaks the wire protocol, but does no I/O. The network server feeds it
//! decoded messages and a 60 Hz tick and sends whatever comes out; the
//! browser build runs the same room in a worker for offline play and the
//! Netcode Lab.

use fb_bot::Bot;
use fb_net::{ClientMessage, PROTOCOL_VERSION, ServerMessage, blob_chunks};
use fb_replay::{Recorder, Replay};
use fb_rollback::bisect::first_divergent_frame;
use fb_rollback::{Authority, Frame};
use fb_sim::{MatchConfig, Pcg32, World};

#[derive(Clone, Copy, Debug)]
pub struct RoomOptions {
    /// Template for every match; the seed is replaced per match.
    pub config: MatchConfig,
    pub checkpoint_interval: Frame,
    /// Ticks to wait between the end of one match and the next.
    pub intermission_ticks: u32,
}

impl Default for RoomOptions {
    fn default() -> Self {
        RoomOptions {
            config: MatchConfig::default(),
            checkpoint_interval: 30,
            intermission_ticks: 5 * 60,
        }
    }
}

/// A message for one player. Reliable messages must arrive, in order;
/// the rest are per-tick and may be lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub player: usize,
    pub reliable: bool,
    pub message: ServerMessage,
}

pub struct FinishedMatch {
    pub id: u64,
    pub replay: Replay,
    pub scores: Vec<u16>,
}

enum Seat {
    Bot(Bot),
    Human { ping: u16 },
}

enum Phase {
    Playing,
    Intermission { ticks_left: u32 },
}

pub struct Room {
    options: RoomOptions,
    rng: Pcg32,
    seats: Vec<Seat>,
    authority: Authority<World>,
    recorder: Recorder,
    config: MatchConfig,
    phase: Phase,
    next_blob: u16,
    matches_played: u64,
    /// Which match this is, modulo 256. Stamped on per-tick messages.
    epoch: u8,
}

impl Room {
    pub fn new(seed: u64, options: RoomOptions) -> Self {
        let mut rng = Pcg32::new(seed);
        let players = options.config.players as usize;
        let seats = (0..players)
            .map(|_| Seat::Bot(Bot::new(rng.next_u32() as u64)))
            .collect();
        let config = MatchConfig {
            seed: next_seed(&mut rng),
            ..options.config
        };
        Room {
            options,
            seats,
            authority: Authority::new(World::new(config), players, options.checkpoint_interval),
            recorder: Recorder::new(config, options.checkpoint_interval),
            config,
            phase: Phase::Playing,
            next_blob: 0,
            matches_played: 0,
            epoch: 0,
            rng,
        }
    }

    pub fn players(&self) -> usize {
        self.seats.len()
    }

    pub fn humans(&self) -> usize {
        self.seats
            .iter()
            .filter(|seat| matches!(seat, Seat::Human { .. }))
            .count()
    }

    pub fn authority(&self) -> &Authority<World> {
        &self.authority
    }

    pub fn world(&self) -> &World {
        self.authority.state()
    }

    pub fn config(&self) -> &MatchConfig {
        &self.config
    }

    /// Seats a new player in place of a bot. Returns `None` if every seat
    /// is taken by a human. The messages bring the client to the present.
    pub fn join(&mut self, out: &mut Vec<Outgoing>) -> Option<usize> {
        let player = self
            .seats
            .iter()
            .position(|seat| matches!(seat, Seat::Bot(_)))?;
        self.seats[player] = Seat::Human { ping: 0 };
        self.welcome(player, out);
        Some(player)
    }

    /// Hands the seat back to a bot, which carries on from wherever the
    /// player was standing.
    pub fn leave(&mut self, player: usize) {
        self.seats[player] = Seat::Bot(Bot::new(self.rng.next_u32() as u64));
    }

    fn welcome(&mut self, player: usize, out: &mut Vec<Outgoing>) {
        out.push(Outgoing {
            player,
            reliable: true,
            message: ServerMessage::Welcome {
                protocol: PROTOCOL_VERSION,
                epoch: self.epoch,
                player: player as u8,
                config: self.config,
            },
        });
        self.send_history(player, out);
    }

    /// Sends the match so far. The client replays it to reach the current
    /// frame, so its acknowledgement restarts from here.
    fn send_history(&mut self, player: usize, out: &mut Vec<Outgoing>) {
        let id = self.next_blob;
        self.next_blob = self.next_blob.wrapping_add(1);
        let history = self.recorder.replay().encode();
        out.extend(
            blob_chunks(id, &history)
                .into_iter()
                .map(|message| Outgoing {
                    player,
                    reliable: true,
                    message,
                }),
        );
        self.authority.snapshot_sent(player);
    }

    pub fn receive(&mut self, player: usize, message: ClientMessage, out: &mut Vec<Outgoing>) {
        let Seat::Human { ping } = &mut self.seats[player] else {
            return;
        };
        match message {
            ClientMessage::Inputs {
                epoch,
                ping: stamp,
                packet,
            } => {
                *ping = stamp;
                if matches!(self.phase, Phase::Playing) && epoch == self.epoch {
                    self.authority.receive(player, &packet);
                }
            }
            ClientMessage::Desync { checksums } => {
                let ours = self.authority.recent_checksums();
                // If the histories no longer overlap, the best statement is
                // the oldest frame the client reported.
                let frame = first_divergent_frame(&checksums, &ours)
                    .or(checksums.first().map(|c| c.0))
                    .unwrap_or(self.authority.frame());
                out.push(Outgoing {
                    player,
                    reliable: true,
                    message: ServerMessage::DesyncFound { frame },
                });
                self.send_history(player, out);
            }
        }
    }

    /// Advances one tick. Returns the finished match on the tick it ends.
    pub fn tick(&mut self, out: &mut Vec<Outgoing>) -> Option<FinishedMatch> {
        if let Phase::Intermission { ticks_left } = &mut self.phase {
            *ticks_left -= 1;
            if *ticks_left == 0 {
                self.start_match(out);
            }
            return None;
        }

        let frame = self.authority.frame();
        for (player, seat) in self.seats.iter_mut().enumerate() {
            if let Seat::Bot(bot) = seat {
                let input = bot.decide(self.authority.state(), player);
                self.authority.offer(player, frame, input);
            }
        }
        let inputs = self.authority.tick().to_vec();
        self.recorder.record(&inputs, self.authority.state());

        for player in 0..self.seats.len() {
            let Seat::Human { ping } = self.seats[player] else {
                continue;
            };
            match self.authority.packet_for(player) {
                Some(packet) => out.push(Outgoing {
                    player,
                    reliable: false,
                    message: ServerMessage::Frames {
                        epoch: self.epoch,
                        pong: ping,
                        packet,
                    },
                }),
                // Silent for longer than the input history reaches back.
                None => self.send_history(player, out),
            }
        }

        if !self.authority.state().finished() {
            return None;
        }
        let world = self.authority.state();
        let scores: Vec<u16> = (0..self.seats.len())
            .map(|player| world.player(player).score)
            .collect();
        self.matches_played += 1;
        let id = self.config.seed ^ self.matches_played.rotate_left(48);
        for (player, seat) in self.seats.iter().enumerate() {
            if matches!(seat, Seat::Human { .. }) {
                out.push(Outgoing {
                    player,
                    reliable: true,
                    message: ServerMessage::MatchOver {
                        scores: scores.clone(),
                        replay_id: id,
                    },
                });
            }
        }
        self.phase = Phase::Intermission {
            ticks_left: self.options.intermission_ticks.max(1),
        };
        let finished = std::mem::replace(
            &mut self.recorder,
            Recorder::new(self.config, self.options.checkpoint_interval),
        );
        Some(FinishedMatch {
            id,
            replay: finished.finish(),
            scores,
        })
    }

    fn start_match(&mut self, out: &mut Vec<Outgoing>) {
        self.config = MatchConfig {
            seed: next_seed(&mut self.rng),
            ..self.options.config
        };
        self.authority = Authority::new(
            World::new(self.config),
            self.seats.len(),
            self.options.checkpoint_interval,
        );
        self.recorder = Recorder::new(self.config, self.options.checkpoint_interval);
        self.phase = Phase::Playing;
        self.epoch = self.epoch.wrapping_add(1);
        for player in 0..self.seats.len() {
            if matches!(self.seats[player], Seat::Human { .. }) {
                self.welcome(player, out);
            }
        }
    }
}

fn next_seed(rng: &mut Pcg32) -> u64 {
    (rng.next_u32() as u64) << 32 | rng.next_u32() as u64
}
