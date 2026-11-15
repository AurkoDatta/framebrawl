//! The client's half of the protocol, with no I/O.
//!
//! Feed it the bytes that arrive and a 60 Hz tick with the local input; it
//! hands back bytes to send and exposes the predicted world. The browser
//! wraps this in a worker; tests drive it directly.

use std::collections::VecDeque;

use fb_net::{BlobAssembler, ClientMessage, DecodeError, PROTOCOL_VERSION, ServerMessage};
use fb_replay::{Replay, ReplayError};
use fb_rollback::{Frame, Rollback, Session, SessionConfig, Stats};
use fb_sim::{Input, MAX_PLAYERS, MatchConfig, World};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The server assigned a seat; the match history follows.
    Welcome {
        player: u8,
        config: MatchConfig,
    },
    /// The history has been replayed and prediction has started.
    Synced {
        frame: Frame,
    },
    Rollback(Rollback),
    /// Our state disagreed with the server's checkpoint at `frame`.
    Desync {
        frame: Frame,
    },
    /// The server located where the desync began.
    DesyncFound {
        frame: Frame,
    },
    MatchOver {
        scores: Vec<u16>,
        replay_id: u64,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum ClientError {
    Decode(DecodeError),
    ProtocolMismatch {
        ours: u16,
        theirs: u16,
    },
    /// The history the server sent does not replay to its own checksums
    /// on this machine: the build is not deterministic here.
    History(ReplayError),
    Unexpected(&'static str),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Decode(e) => write!(f, "bad message from server: {e}"),
            ClientError::ProtocolMismatch { ours, theirs } => {
                write!(f, "server speaks protocol {theirs}, this client {ours}")
            }
            ClientError::History(e) => write!(f, "match history did not replay: {e}"),
            ClientError::Unexpected(what) => write!(f, "unexpected {what}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A message to send, and whether it needs the reliable channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outgoing {
    pub reliable: bool,
    pub bytes: Vec<u8>,
}

struct Seat {
    epoch: u8,
    player: u8,
    config: MatchConfig,
    /// `None` until the history has been replayed.
    session: Option<Session<World>>,
    desync_reported: bool,
}

pub struct Client {
    session_config: SessionConfig,
    seat: Option<Seat>,
    assembler: BlobAssembler,
    events: VecDeque<Event>,
    /// Smoothed round-trip time in 1/16 ms.
    rtt_x16: u32,
}

impl Client {
    pub fn new(session_config: SessionConfig) -> Self {
        Client {
            session_config,
            seat: None,
            assembler: BlobAssembler::default(),
            events: VecDeque::new(),
            rtt_x16: 0,
        }
    }

    pub fn player(&self) -> Option<u8> {
        self.seat.as_ref().map(|seat| seat.player)
    }

    pub fn config(&self) -> Option<&MatchConfig> {
        self.seat.as_ref().map(|seat| &seat.config)
    }

    pub fn session(&self) -> Option<&Session<World>> {
        self.seat.as_ref()?.session.as_ref()
    }

    /// The predicted present, once synced.
    pub fn world(&self) -> Option<&World> {
        self.session().map(Session::state)
    }

    pub fn frame(&self) -> Frame {
        self.session().map_or(0, Session::frame)
    }

    pub fn confirmed_frame(&self) -> Frame {
        self.session().map_or(0, Session::confirmed_frame)
    }

    pub fn stats(&self) -> Stats {
        self.session().map(Session::stats).unwrap_or_default()
    }

    pub fn input_delay(&self) -> u8 {
        self.session().map_or(0, Session::input_delay)
    }

    /// Smoothed round trip in milliseconds, including up to one tick of
    /// waiting on the server. Zero until the first echo.
    pub fn rtt_ms(&self) -> u32 {
        self.rtt_x16 / 16
    }

    /// Corrupts the local simulation so that it no longer matches the
    /// server's. See [`Session::tamper`].
    pub fn corrupt(&mut self) {
        if let Some(seat) = self.seat.as_mut()
            && let Some(session) = seat.session.as_mut()
        {
            let player = usize::from(seat.player);
            session.tamper(|world| world.corrupt(player));
        }
    }

    pub fn next_event(&mut self) -> Option<Event> {
        self.events.pop_front()
    }

    /// Handles one message from the server. `now_ms` is any millisecond
    /// clock; only differences matter.
    pub fn receive(&mut self, bytes: &[u8], now_ms: u32) -> Result<(), ClientError> {
        let players = self
            .seat
            .as_ref()
            .map_or(MAX_PLAYERS, |seat| seat.config.players as usize);
        match ServerMessage::decode(bytes, players).map_err(ClientError::Decode)? {
            ServerMessage::Welcome {
                protocol,
                epoch,
                player,
                config,
            } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(ClientError::ProtocolMismatch {
                        ours: PROTOCOL_VERSION,
                        theirs: protocol,
                    });
                }
                self.seat = Some(Seat {
                    epoch,
                    player,
                    config,
                    session: None,
                    desync_reported: false,
                });
                self.events.push_back(Event::Welcome { player, config });
            }
            ServerMessage::Blob {
                id,
                index,
                total,
                bytes,
            } => {
                if let Some(history) = self.assembler.push(id, index, total, &bytes) {
                    self.adopt_history(&history)?;
                }
            }
            ServerMessage::Frames {
                epoch,
                pong,
                packet,
            } => {
                self.measure_rtt(pong, now_ms);
                // Frames from a match that has ended are stale. Frames that
                // arrive before the history is replayed are dropped; the
                // server resends from our acknowledgement.
                if let Some(seat) = self.seat.as_mut()
                    && seat.epoch == epoch
                    && let Some(session) = seat.session.as_mut()
                    && let Some(rollback) = session.receive(&packet)
                {
                    self.events.push_back(Event::Rollback(rollback));
                }
            }
            ServerMessage::DesyncFound { frame } => {
                self.events.push_back(Event::DesyncFound { frame });
            }
            ServerMessage::MatchOver { scores, replay_id } => {
                self.events
                    .push_back(Event::MatchOver { scores, replay_id });
            }
        }
        Ok(())
    }

    /// Replays the match so far and starts (or restarts) prediction from
    /// its last frame.
    fn adopt_history(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        let seat = self
            .seat
            .as_mut()
            .ok_or(ClientError::Unexpected("history before welcome"))?;
        let replay = Replay::decode(bytes).map_err(ClientError::History)?;
        if replay.config != seat.config {
            return Err(ClientError::Unexpected("history for another match"));
        }
        let world = replay.simulate().map_err(ClientError::History)?;
        let frame = replay.frames();
        let players = seat.config.players as usize;
        let last_inputs = match frame.checked_sub(1) {
            Some(last) => replay.row(last).to_vec(),
            None => vec![Input::default(); players],
        };
        match &mut seat.session {
            Some(session) => session.resync(frame, world, &last_inputs),
            None => {
                let mut session = Session::new(
                    world.clone(),
                    players,
                    seat.player as usize,
                    self.session_config,
                );
                session.resync(frame, world, &last_inputs);
                seat.session = Some(session);
            }
        }
        seat.desync_reported = false;
        self.events.push_back(Event::Synced { frame });
        Ok(())
    }

    fn measure_rtt(&mut self, pong: u16, now_ms: u32) {
        let sample = (now_ms as u16).wrapping_sub(pong) as u32;
        // An echo of a stamp from before a long pause is not a measurement.
        if sample > 5000 {
            return;
        }
        self.rtt_x16 = if self.rtt_x16 == 0 {
            sample * 16
        } else {
            (self.rtt_x16 * 7 + sample * 16) / 8
        };
    }

    /// One 60 Hz timer tick: simulates as many frames as pacing calls for
    /// with `local` as the input, and returns what to send.
    pub fn tick(&mut self, local: Input, now_ms: u32) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let Some(seat) = self.seat.as_mut() else {
            return out;
        };
        let Some(session) = seat.session.as_mut() else {
            return out;
        };

        for _ in 0..session.frames_due() {
            session.advance(local);
        }
        out.push(Outgoing {
            reliable: false,
            bytes: ClientMessage::Inputs {
                epoch: seat.epoch,
                ping: now_ms as u16,
                packet: session.outgoing(),
            }
            .encode(),
        });

        if let Some(desync) = session.desync()
            && !seat.desync_reported
        {
            seat.desync_reported = true;
            self.events.push_back(Event::Desync {
                frame: desync.frame,
            });
            out.push(Outgoing {
                reliable: true,
                bytes: ClientMessage::Desync {
                    checksums: session.recent_checksums(),
                }
                .encode(),
            });
        }
        out
    }
}
