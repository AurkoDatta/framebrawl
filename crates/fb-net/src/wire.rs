//! The byte format of everything that crosses the network.
//!
//! Per-tick messages (`Inputs`, `Frames`) are bit-packed and sent on the
//! unreliable channel. Inputs are 12 bits each, and a value equal to the
//! one before it costs a single bit, which is the common case: players
//! hold a direction for many frames. The rest are rare and go on the
//! reliable channel.
//!
//! Decoding never trusts its input: every length is bounded and every read
//! is checked, so a malformed packet is an error rather than a panic or a
//! large allocation.

use fb_rollback::{ClientPacket, Frame, MAX_FRAMES_PER_PACKET, ServerPacket};
use fb_sim::{Input, MAX_PLAYERS, MatchConfig};

use crate::bits::{BitReader, BitWriter, OutOfData};

/// Bumped whenever the format or the simulation changes incompatibly.
pub const PROTOCOL_VERSION: u16 = 1;

/// Largest payload carried in one `Blob` message. Small enough for any
/// data channel implementation's message limit.
pub const BLOB_CHUNK: usize = 12_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMessage {
    /// Sent every tick. `epoch` names the match the frames belong to, so
    /// packets still in flight when a match ends cannot leak into the
    /// next. `ping` is an opaque stamp the server echoes back.
    Inputs {
        epoch: u8,
        ping: u16,
        packet: ClientPacket,
    },
    /// The client's state disagreed with a checkpoint. Carries its recent
    /// per-frame checksums so the server can find where it began.
    Desync { checksums: Vec<(Frame, u64)> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMessage {
    /// First message on a connection: which player this client is.
    Welcome {
        protocol: u16,
        epoch: u8,
        player: u8,
        config: MatchConfig,
    },
    /// Sent every tick. `pong` echoes the newest `ping` received.
    Frames {
        epoch: u8,
        pong: u16,
        packet: ServerPacket,
    },
    /// One piece of a larger payload (the input history a joining or
    /// desynced client replays to reach the present).
    Blob {
        id: u16,
        index: u16,
        total: u16,
        bytes: Vec<u8>,
    },
    /// The server's verdict on a desync report.
    DesyncFound {
        frame: Frame,
    },
    MatchOver {
        scores: Vec<u16>,
        replay_id: u64,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    Truncated,
    UnknownKind(u8),
    Invalid(&'static str),
}

impl From<OutOfData> for DecodeError {
    fn from(_: OutOfData) -> Self {
        DecodeError::Truncated
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "message ends early"),
            DecodeError::UnknownKind(kind) => write!(f, "unknown message kind {kind}"),
            DecodeError::Invalid(what) => write!(f, "invalid {what}"),
        }
    }
}

impl std::error::Error for DecodeError {}

const KIND_BITS: u8 = 3;
const COUNT_BITS: u8 = 6;
const MAX_DESYNC_CHECKSUMS: usize = 63;

fn put_input(w: &mut BitWriter, input: Input) {
    w.put(input.buttons as u64, 4);
    w.put(input.aim as u64, 8);
}

fn get_input(r: &mut BitReader) -> Result<Input, DecodeError> {
    let buttons = r.get(4)? as u8;
    let aim = r.get(8)? as u8;
    Ok(Input::new(buttons, aim))
}

/// Writes `input`, or a single bit if it equals `previous`.
fn put_input_delta(w: &mut BitWriter, previous: &mut Option<Input>, input: Input) {
    let repeat = *previous == Some(input);
    w.put_bool(repeat);
    if !repeat {
        put_input(w, input);
    }
    *previous = Some(input);
}

fn get_input_delta(r: &mut BitReader, previous: &mut Option<Input>) -> Result<Input, DecodeError> {
    let input = if r.get_bool()? {
        previous.ok_or(DecodeError::Invalid("repeat with nothing to repeat"))?
    } else {
        get_input(r)?
    };
    *previous = Some(input);
    Ok(input)
}

impl ClientMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = BitWriter::new();
        match self {
            ClientMessage::Inputs {
                epoch,
                ping,
                packet,
            } => {
                w.put(0, KIND_BITS);
                w.put(*epoch as u64, 8);
                w.put(*ping as u64, 16);
                w.put(packet.ack as u64, 32);
                w.put(packet.first_frame as u64, 32);
                let count = packet.inputs.len().min(MAX_FRAMES_PER_PACKET);
                w.put(count as u64, COUNT_BITS);
                let mut previous = None;
                for &input in &packet.inputs[..count] {
                    put_input_delta(&mut w, &mut previous, input);
                }
            }
            ClientMessage::Desync { checksums } => {
                w.put(1, KIND_BITS);
                let count = checksums.len().min(MAX_DESYNC_CHECKSUMS);
                w.put(count as u64, COUNT_BITS);
                // The newest entries are the ones that matter.
                for &(frame, checksum) in &checksums[checksums.len() - count..] {
                    w.put(frame as u64, 32);
                    w.put(checksum, 64);
                }
            }
        }
        w.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = BitReader::new(bytes);
        match r.get(KIND_BITS)? as u8 {
            0 => {
                let epoch = r.get(8)? as u8;
                let ping = r.get(16)? as u16;
                let ack = r.get(32)? as Frame;
                let first_frame = r.get(32)? as Frame;
                let count = r.get(COUNT_BITS)? as usize;
                if count > MAX_FRAMES_PER_PACKET {
                    return Err(DecodeError::Invalid("input count"));
                }
                let mut previous = None;
                let inputs = (0..count)
                    .map(|_| get_input_delta(&mut r, &mut previous))
                    .collect::<Result<_, _>>()?;
                Ok(ClientMessage::Inputs {
                    epoch,
                    ping,
                    packet: ClientPacket {
                        ack,
                        first_frame,
                        inputs,
                    },
                })
            }
            1 => {
                let count = r.get(COUNT_BITS)? as usize;
                let checksums = (0..count)
                    .map(|_| Ok((r.get(32)? as Frame, r.get(64)?)))
                    .collect::<Result<_, DecodeError>>()?;
                Ok(ClientMessage::Desync { checksums })
            }
            kind => Err(DecodeError::UnknownKind(kind)),
        }
    }
}

impl ServerMessage {
    /// `players` is the number of inputs per row in a `Frames` packet.
    pub fn encode(&self, players: usize) -> Vec<u8> {
        let mut w = BitWriter::new();
        match self {
            ServerMessage::Welcome {
                protocol,
                epoch,
                player,
                config,
            } => {
                w.put(0, KIND_BITS);
                w.put(*protocol as u64, 16);
                w.put(*epoch as u64, 8);
                w.put(*player as u64, 8);
                w.put_bytes(&config.to_bytes());
            }
            ServerMessage::Frames {
                epoch,
                pong,
                packet,
            } => {
                w.put(1, KIND_BITS);
                w.put(*epoch as u64, 8);
                w.put(*pong as u64, 16);
                w.put(packet.first_frame as u64, 32);
                w.put(packet.lead as u8 as u64, 8);
                let frames = packet.frames(players).min(MAX_FRAMES_PER_PACKET);
                w.put(frames as u64, COUNT_BITS);
                // Each player's column is delta-coded against the row above.
                let mut previous = [None; MAX_PLAYERS];
                for row in 0..frames {
                    for (player, &input) in packet.row(players, row).iter().enumerate() {
                        put_input_delta(&mut w, &mut previous[player], input);
                    }
                }
                w.put_bool(packet.checkpoint.is_some());
                if let Some((frame, checksum)) = packet.checkpoint {
                    w.put(frame as u64, 32);
                    w.put(checksum, 64);
                }
            }
            ServerMessage::Blob {
                id,
                index,
                total,
                bytes,
            } => {
                w.put(2, KIND_BITS);
                w.put(*id as u64, 16);
                w.put(*index as u64, 16);
                w.put(*total as u64, 16);
                w.put_bytes(bytes);
            }
            ServerMessage::DesyncFound { frame } => {
                w.put(3, KIND_BITS);
                w.put(*frame as u64, 32);
            }
            ServerMessage::MatchOver { scores, replay_id } => {
                w.put(4, KIND_BITS);
                w.put(*replay_id, 64);
                w.put(scores.len().min(MAX_PLAYERS) as u64, 3);
                for &score in scores.iter().take(MAX_PLAYERS) {
                    w.put(score as u64, 16);
                }
            }
        }
        w.finish()
    }

    pub fn decode(bytes: &[u8], players: usize) -> Result<Self, DecodeError> {
        if players == 0 || players > MAX_PLAYERS {
            return Err(DecodeError::Invalid("player count"));
        }
        let mut r = BitReader::new(bytes);
        match r.get(KIND_BITS)? as u8 {
            0 => {
                let protocol = r.get(16)? as u16;
                let epoch = r.get(8)? as u8;
                let player = r.get(8)? as u8;
                let config = MatchConfig::from_bytes(r.rest())
                    .map_err(|_| DecodeError::Invalid("match config"))?;
                if player >= config.players {
                    return Err(DecodeError::Invalid("player index"));
                }
                Ok(ServerMessage::Welcome {
                    protocol,
                    epoch,
                    player,
                    config,
                })
            }
            1 => {
                let epoch = r.get(8)? as u8;
                let pong = r.get(16)? as u16;
                let first_frame = r.get(32)? as Frame;
                let lead = r.get(8)? as u8 as i8;
                let frames = r.get(COUNT_BITS)? as usize;
                if frames > MAX_FRAMES_PER_PACKET {
                    return Err(DecodeError::Invalid("frame count"));
                }
                let mut previous = [None; MAX_PLAYERS];
                let mut inputs = Vec::with_capacity(frames * players);
                for _ in 0..frames {
                    for column in previous.iter_mut().take(players) {
                        inputs.push(get_input_delta(&mut r, column)?);
                    }
                }
                let checkpoint = if r.get_bool()? {
                    Some((r.get(32)? as Frame, r.get(64)?))
                } else {
                    None
                };
                Ok(ServerMessage::Frames {
                    epoch,
                    pong,
                    packet: ServerPacket {
                        first_frame,
                        inputs,
                        lead,
                        checkpoint,
                    },
                })
            }
            2 => {
                let id = r.get(16)? as u16;
                let index = r.get(16)? as u16;
                let total = r.get(16)? as u16;
                let bytes = r.rest();
                if index >= total || bytes.len() > BLOB_CHUNK {
                    return Err(DecodeError::Invalid("blob chunk"));
                }
                Ok(ServerMessage::Blob {
                    id,
                    index,
                    total,
                    bytes: bytes.to_vec(),
                })
            }
            3 => Ok(ServerMessage::DesyncFound {
                frame: r.get(32)? as Frame,
            }),
            4 => {
                let replay_id = r.get(64)?;
                let count = r.get(3)? as usize;
                if count > MAX_PLAYERS {
                    return Err(DecodeError::Invalid("score count"));
                }
                let scores = (0..count)
                    .map(|_| Ok(r.get(16)? as u16))
                    .collect::<Result<_, DecodeError>>()?;
                Ok(ServerMessage::MatchOver { scores, replay_id })
            }
            kind => Err(DecodeError::UnknownKind(kind)),
        }
    }
}

/// Splits a payload into `Blob` messages.
pub fn blob_chunks(id: u16, payload: &[u8]) -> Vec<ServerMessage> {
    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&[]]
    } else {
        payload.chunks(BLOB_CHUNK).collect()
    };
    let total = chunks.len() as u16;
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, bytes)| ServerMessage::Blob {
            id,
            index: index as u16,
            total,
            bytes: bytes.to_vec(),
        })
        .collect()
}

/// Reassembles `Blob` messages arriving in order on the reliable channel.
#[derive(Default)]
pub struct BlobAssembler {
    id: u16,
    next: u16,
    bytes: Vec<u8>,
}

impl BlobAssembler {
    /// Feeds one chunk; returns the whole payload when the last arrives.
    /// A chunk that does not continue the current blob starts over.
    pub fn push(&mut self, id: u16, index: u16, total: u16, bytes: &[u8]) -> Option<Vec<u8>> {
        if index == 0 {
            self.id = id;
            self.next = 0;
            self.bytes.clear();
        }
        if id != self.id || index != self.next {
            self.next = u16::MAX;
            return None;
        }
        self.bytes.extend_from_slice(bytes);
        self.next += 1;
        (self.next == total).then(|| std::mem::take(&mut self.bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fb_sim::Pcg32;

    fn random_input(rng: &mut Pcg32, previous: Option<Input>) -> Input {
        match previous {
            Some(input) if rng.below(4) != 0 => input,
            _ => Input::new(rng.below(16) as u8, rng.below(256) as u8),
        }
    }

    fn random_client_message(rng: &mut Pcg32) -> ClientMessage {
        if rng.below(8) == 0 {
            let count = rng.below(MAX_DESYNC_CHECKSUMS as u32 + 1);
            return ClientMessage::Desync {
                checksums: (0..count)
                    .map(|i| (i, (rng.next_u32() as u64) << 32 | rng.next_u32() as u64))
                    .collect(),
            };
        }
        let mut previous = None;
        let inputs = (0..rng.below(MAX_FRAMES_PER_PACKET as u32 + 1))
            .map(|_| {
                let input = random_input(rng, previous);
                previous = Some(input);
                input
            })
            .collect();
        ClientMessage::Inputs {
            epoch: rng.next_u32() as u8,
            ping: rng.next_u32() as u16,
            packet: ClientPacket {
                ack: rng.next_u32(),
                first_frame: rng.next_u32(),
                inputs,
            },
        }
    }

    fn random_server_message(rng: &mut Pcg32, players: usize) -> ServerMessage {
        match rng.below(10) {
            0 => ServerMessage::Welcome {
                protocol: PROTOCOL_VERSION,
                epoch: rng.next_u32() as u8,
                player: rng.below(players as u32) as u8,
                config: MatchConfig {
                    seed: rng.next_u32() as u64,
                    players: players.max(2) as u8,
                    ..MatchConfig::default()
                },
            },
            1 => ServerMessage::Blob {
                id: rng.next_u32() as u16,
                index: 2,
                total: 5,
                bytes: (0..rng.below(300)).map(|_| rng.next_u32() as u8).collect(),
            },
            2 => ServerMessage::DesyncFound {
                frame: rng.next_u32(),
            },
            3 => ServerMessage::MatchOver {
                scores: (0..players).map(|_| rng.next_u32() as u16).collect(),
                replay_id: (rng.next_u32() as u64) << 32 | rng.next_u32() as u64,
            },
            _ => {
                let frames = rng.below(MAX_FRAMES_PER_PACKET as u32 + 1) as usize;
                let mut inputs: Vec<Input> = Vec::new();
                for i in 0..frames * players {
                    let above = i.checked_sub(players).map(|j| inputs[j]);
                    inputs.push(random_input(rng, above));
                }
                ServerMessage::Frames {
                    epoch: rng.next_u32() as u8,
                    pong: rng.next_u32() as u16,
                    packet: ServerPacket {
                        first_frame: rng.next_u32(),
                        inputs,
                        lead: rng.next_u32() as i8,
                        checkpoint: (rng.below(3) == 0)
                            .then(|| (rng.next_u32(), 0xdead_beef_0bad_f00d)),
                    },
                }
            }
        }
    }

    #[test]
    fn client_messages_round_trip() {
        let mut rng = Pcg32::new(1);
        for _ in 0..2000 {
            let message = random_client_message(&mut rng);
            assert_eq!(ClientMessage::decode(&message.encode()), Ok(message));
        }
    }

    #[test]
    fn server_messages_round_trip_for_every_player_count() {
        let mut rng = Pcg32::new(2);
        for i in 0..3000 {
            let players = 2 + i % 3;
            let message = random_server_message(&mut rng, players);
            assert_eq!(
                ServerMessage::decode(&message.encode(players), players),
                Ok(message)
            );
        }
    }

    #[test]
    fn held_inputs_cost_a_bit_each() {
        let hold = Input::new(Input::MOVE, 40);
        let client = ClientMessage::Inputs {
            epoch: 1,
            ping: 7,
            packet: ClientPacket {
                ack: 1000,
                first_frame: 1000,
                inputs: vec![hold; 12],
            },
        };
        assert_eq!(client.encode().len(), 16);

        let server = ServerMessage::Frames {
            epoch: 1,
            pong: 7,
            packet: ServerPacket {
                first_frame: 1000,
                inputs: vec![hold; 12 * 4],
                lead: 1,
                checkpoint: None,
            },
        };
        assert_eq!(server.encode(4).len(), 22);
    }

    #[test]
    fn garbage_and_truncation_are_errors_never_panics() {
        let mut rng = Pcg32::new(3);
        for _ in 0..20_000 {
            let bytes: Vec<u8> = (0..rng.below(64)).map(|_| rng.next_u32() as u8).collect();
            let _ = ClientMessage::decode(&bytes);
            let _ = ServerMessage::decode(&bytes, 2 + rng.below(3) as usize);
        }
        for _ in 0..500 {
            let message = random_server_message(&mut rng, 4);
            let bytes = message.encode(4);
            for cut in 0..bytes.len() {
                // Any outcome but a panic is fine; a prefix can happen to
                // decode as a shorter valid message.
                let _ = ServerMessage::decode(&bytes[..cut], 4);
            }
        }
        assert_eq!(ClientMessage::decode(&[]), Err(DecodeError::Truncated));
        assert_eq!(
            ClientMessage::decode(&[7]),
            Err(DecodeError::UnknownKind(7))
        );
    }

    #[test]
    fn a_hostile_count_cannot_force_a_large_allocation() {
        // Kind 0, then all ones: count field says 63 inputs.
        let bytes = [
            0xf8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ];
        assert_eq!(
            ClientMessage::decode(&bytes),
            Err(DecodeError::Invalid("input count"))
        );
    }

    #[test]
    fn blobs_split_and_reassemble() {
        let payload: Vec<u8> = (0..30_000u32).map(|i| (i * 7) as u8).collect();
        let chunks = blob_chunks(9, &payload);
        assert_eq!(chunks.len(), 3);
        let mut assembler = BlobAssembler::default();
        let mut whole = None;
        for chunk in &chunks {
            let ServerMessage::Blob {
                id,
                index,
                total,
                bytes,
            } = ServerMessage::decode(&chunk.encode(4), 4).expect("decodes")
            else {
                panic!("not a blob");
            };
            whole = assembler.push(id, index, total, &bytes);
        }
        assert_eq!(whole, Some(payload));
        assert_eq!(
            blob_chunks(1, &[]).len(),
            1,
            "an empty payload still arrives"
        );
    }

    #[test]
    fn a_missing_chunk_never_yields_a_payload() {
        let chunks = blob_chunks(3, &vec![1u8; BLOB_CHUNK * 3]);
        let mut assembler = BlobAssembler::default();
        let mut results = Vec::new();
        for skip in [0usize, 2] {
            let ServerMessage::Blob {
                id,
                index,
                total,
                bytes,
            } = &chunks[skip]
            else {
                unreachable!()
            };
            results.push(assembler.push(*id, *index, *total, bytes));
        }
        assert_eq!(results, [None, None]);
    }
}
