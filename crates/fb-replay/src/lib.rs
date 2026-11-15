//! Replays: a match stored as its config and its authoritative inputs.
//!
//! Because the simulation is deterministic, that is the whole match. A
//! replay also carries periodic state checksums recorded by the server, so
//! playing it back proves the result rather than merely showing it. The
//! same encoding brings a joining or desynced client up to date: it
//! replays the inputs so far and arrives at the server's state.
//!
//! Layout, little-endian:
//!
//! ```text
//! "FBR1"                     magic
//! u16                        format version
//! [u8; 17]                   MatchConfig
//! u32                        frame count
//! per player:  u32 run count, then runs of (buttons u8, aim u8, length varint)
//! u32                        checkpoint count
//! per checkpoint: u32 frame, u64 checksum of the state at that frame
//! ```

use fb_sim::{Input, MatchConfig, World};

pub const FORMAT_VERSION: u16 = 1;
const MAGIC: &[u8; 4] = b"FBR1";
/// An hour of play. Anything longer in a header is treated as corrupt
/// rather than allocated.
const MAX_FRAMES: u32 = 60 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    pub config: MatchConfig,
    /// One row of `config.players` inputs per frame.
    pub inputs: Vec<Input>,
    /// `(frame, checksum)`: the state after all inputs before `frame`.
    pub checkpoints: Vec<(u32, u64)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReplayError {
    NotAReplay,
    UnsupportedVersion(u16),
    Corrupt(&'static str),
    /// Playing the inputs back produced a different state than recorded.
    Diverged {
        frame: u32,
        recorded: u64,
        replayed: u64,
    },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::NotAReplay => write!(f, "not a replay file"),
            ReplayError::UnsupportedVersion(v) => write!(f, "unsupported replay version {v}"),
            ReplayError::Corrupt(what) => write!(f, "corrupt replay: {what}"),
            ReplayError::Diverged {
                frame,
                recorded,
                replayed,
            } => write!(
                f,
                "replay diverged at frame {frame}: recorded {recorded:016x}, replayed {replayed:016x}"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Builds a replay as the server simulates.
pub struct Recorder {
    replay: Replay,
    checkpoint_interval: u32,
}

impl Recorder {
    pub fn new(config: MatchConfig, checkpoint_interval: u32) -> Self {
        assert!(checkpoint_interval > 0);
        Recorder {
            replay: Replay {
                config,
                inputs: Vec::new(),
                checkpoints: Vec::new(),
            },
            checkpoint_interval,
        }
    }

    /// Records the inputs of the frame just simulated; `world` is the
    /// state after it.
    pub fn record(&mut self, inputs: &[Input], world: &World) {
        assert_eq!(inputs.len(), self.replay.config.players as usize);
        self.replay.inputs.extend_from_slice(inputs);
        if world.frame().is_multiple_of(self.checkpoint_interval) {
            self.replay
                .checkpoints
                .push((world.frame(), world.checksum()));
        }
    }

    /// The match so far.
    pub fn replay(&self) -> &Replay {
        &self.replay
    }

    pub fn finish(self) -> Replay {
        self.replay
    }
}

fn put_varint(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], ReplayError> {
        if self.bytes.len() < count {
            return Err(ReplayError::Corrupt("ends early"));
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, ReplayError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ReplayError> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64, ReplayError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }

    fn varint(&mut self) -> Result<u32, ReplayError> {
        let mut value = 0u32;
        for shift in [0, 7, 14, 21, 28] {
            let byte = self.u8()?;
            value |= ((byte & 0x7f) as u32) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(ReplayError::Corrupt("run length"))
    }
}

impl Replay {
    pub fn frames(&self) -> u32 {
        (self.inputs.len() / self.config.players as usize) as u32
    }

    pub fn row(&self, frame: u32) -> &[Input] {
        let players = self.config.players as usize;
        &self.inputs[frame as usize * players..(frame as usize + 1) * players]
    }

    pub fn encode(&self) -> Vec<u8> {
        let players = self.config.players as usize;
        let mut out = Vec::new();
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.config.to_bytes());
        out.extend_from_slice(&self.frames().to_le_bytes());

        for player in 0..players {
            let mut runs: Vec<(Input, u32)> = Vec::new();
            for input in self.inputs.iter().skip(player).step_by(players) {
                match runs.last_mut() {
                    Some((held, length)) if held == input => *length += 1,
                    _ => runs.push((*input, 1)),
                }
            }
            out.extend_from_slice(&(runs.len() as u32).to_le_bytes());
            for (input, length) in runs {
                out.extend_from_slice(&input.to_bytes());
                put_varint(&mut out, length);
            }
        }

        out.extend_from_slice(&(self.checkpoints.len() as u32).to_le_bytes());
        for &(frame, checksum) in &self.checkpoints {
            out.extend_from_slice(&frame.to_le_bytes());
            out.extend_from_slice(&checksum.to_le_bytes());
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ReplayError> {
        let mut r = Reader { bytes };
        if r.take(4).map_err(|_| ReplayError::NotAReplay)? != MAGIC {
            return Err(ReplayError::NotAReplay);
        }
        let version = u16::from_le_bytes(r.take(2)?.try_into().expect("2 bytes"));
        if version != FORMAT_VERSION {
            return Err(ReplayError::UnsupportedVersion(version));
        }
        let config = MatchConfig::from_bytes(r.take(MatchConfig::BYTES)?)
            .map_err(|_| ReplayError::Corrupt("match config"))?;
        let players = config.players as usize;
        let frames = r.u32()?;
        if frames > MAX_FRAMES {
            return Err(ReplayError::Corrupt("frame count"));
        }

        let mut inputs = vec![Input::default(); frames as usize * players];
        for player in 0..players {
            let runs = r.u32()?;
            let mut frame = 0u32;
            for _ in 0..runs {
                let input = Input::from_bytes([r.u8()?, r.u8()?]);
                let length = r.varint()?;
                let end = frame
                    .checked_add(length)
                    .filter(|&end| end <= frames && length > 0)
                    .ok_or(ReplayError::Corrupt("run length"))?;
                for f in frame..end {
                    inputs[f as usize * players + player] = input;
                }
                frame = end;
            }
            if frame != frames {
                return Err(ReplayError::Corrupt("input stream shorter than the match"));
            }
        }

        let count = r.u32()?;
        if count > frames {
            return Err(ReplayError::Corrupt("checkpoint count"));
        }
        let mut checkpoints = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let frame = r.u32()?;
            if frame > frames || checkpoints.last().is_some_and(|&(last, _)| frame <= last) {
                return Err(ReplayError::Corrupt("checkpoint order"));
            }
            checkpoints.push((frame, r.u64()?));
        }
        if !r.bytes.is_empty() {
            return Err(ReplayError::Corrupt("trailing bytes"));
        }
        Ok(Replay {
            config,
            inputs,
            checkpoints,
        })
    }

    /// Plays the inputs back from the start, checking every recorded
    /// checkpoint, and returns the final state.
    pub fn simulate(&self) -> Result<World, ReplayError> {
        let mut world = World::new(self.config);
        let mut checkpoints = self.checkpoints.iter().peekable();
        for frame in 0..self.frames() {
            world.step(self.row(frame));
            if let Some(&&(at, recorded)) = checkpoints.peek()
                && at == world.frame()
            {
                checkpoints.next();
                let replayed = world.checksum();
                if replayed != recorded {
                    return Err(ReplayError::Diverged {
                        frame: at,
                        recorded,
                        replayed,
                    });
                }
            }
        }
        Ok(world)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fb_sim::Pcg32;
    use fb_sim::soak::InputScript;

    fn record(seed: u64, frames: u32) -> (Replay, World) {
        let config = MatchConfig {
            seed,
            ..MatchConfig::default()
        };
        let mut world = World::new(config);
        let mut script = InputScript::new(seed, 4);
        let mut recorder = Recorder::new(config, 30);
        for _ in 0..frames {
            let inputs = script.next_frame(&world);
            world.step(&inputs);
            recorder.record(&inputs, &world);
        }
        (recorder.finish(), world)
    }

    #[test]
    fn a_recorded_match_round_trips_and_replays_to_the_same_state() {
        let (replay, world) = record(1, 2000);
        assert_eq!(replay.checkpoints.len(), 66);
        let bytes = replay.encode();
        let decoded = Replay::decode(&bytes).expect("decodes");
        assert_eq!(decoded, replay);
        let replayed = decoded.simulate().expect("verifies");
        assert_eq!(replayed.checksum(), world.checksum());
    }

    #[test]
    fn a_minute_of_four_players_is_a_few_kilobytes() {
        let (replay, _) = record(2, 3600);
        let size = replay.encode().len();
        let raw = 3600 * 4 * 2;
        assert!(size < raw / 4, "{size} bytes against {raw} raw");
    }

    #[test]
    fn a_changed_input_is_reported_at_the_next_checkpoint() {
        let (mut replay, _) = record(3, 600);
        // Flip one player's input in frame 100.
        let slot = 100 * 4 + 1;
        replay.inputs[slot] = Input::new(replay.inputs[slot].buttons ^ Input::MOVE, 200);
        match replay.simulate() {
            Err(ReplayError::Diverged { frame, .. }) => assert_eq!(frame, 120),
            other => panic!("expected divergence, got {:?}", other.map(|w| w.frame())),
        }
    }

    #[test]
    fn an_unfinished_match_replays_to_the_present() {
        // The resync path: the replay so far, with no final checkpoint.
        let (replay, world) = record(4, 47);
        assert_eq!(replay.checkpoints.len(), 1);
        let rebuilt = Replay::decode(&replay.encode())
            .expect("decodes")
            .simulate()
            .expect("verifies");
        assert_eq!(
            (rebuilt.frame(), rebuilt.checksum()),
            (47, world.checksum())
        );

        let (empty, _) = record(4, 0);
        assert_eq!(
            Replay::decode(&empty.encode()).expect("decodes").frames(),
            0
        );
    }

    #[test]
    fn corrupt_files_are_rejected_without_panicking() {
        let (replay, _) = record(5, 300);
        let bytes = replay.encode();
        assert_eq!(Replay::decode(b"nope"), Err(ReplayError::NotAReplay));
        assert_eq!(Replay::decode(&[]), Err(ReplayError::NotAReplay));
        for cut in 0..bytes.len() {
            assert!(Replay::decode(&bytes[..cut]).is_err(), "truncated at {cut}");
        }
        let mut rng = Pcg32::new(5);
        for _ in 0..5000 {
            let mut damaged = bytes.clone();
            let at = rng.below(damaged.len() as u32) as usize;
            damaged[at] ^= 1 << rng.below(8);
            // Either rejected, or decodes to a different match that then
            // fails verification or is a legitimately different input.
            let _ = Replay::decode(&damaged);
        }

        let mut huge = bytes.clone();
        huge[23..27].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            Replay::decode(&huge),
            Err(ReplayError::Corrupt("frame count"))
        );
    }
}
