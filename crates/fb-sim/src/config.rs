use serde::{Deserialize, Serialize};

pub const MAX_PLAYERS: usize = 4;

/// Everything that distinguishes one match from another. Two worlds built
/// from equal configs and fed equal inputs stay equal forever.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchConfig {
    pub seed: u64,
    pub players: u8,
    /// Tiles along each side of the square arena.
    pub grid: u8,
    pub props: u8,
    pub duration_frames: u32,
    pub score_limit: u16,
}

impl Default for MatchConfig {
    fn default() -> Self {
        MatchConfig {
            seed: 0,
            players: 4,
            grid: 9,
            props: 10,
            duration_frames: 60 * 150,
            score_limit: 5,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ConfigError {
    Players(u8),
    Grid(u8),
    Truncated,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Players(n) => write!(f, "player count {n} is outside 2..=4"),
            ConfigError::Grid(n) => write!(f, "grid size {n} is outside 4..=16"),
            ConfigError::Truncated => {
                write!(f, "config is shorter than {} bytes", MatchConfig::BYTES)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl MatchConfig {
    pub const BYTES: usize = 17;

    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(2..=MAX_PLAYERS as u8).contains(&self.players) {
            return Err(ConfigError::Players(self.players));
        }
        if !(4..=16).contains(&self.grid) {
            return Err(ConfigError::Grid(self.grid));
        }
        Ok(())
    }

    /// Little-endian, fixed order. This layout is what replays store.
    pub fn to_bytes(&self) -> [u8; Self::BYTES] {
        let mut out = [0; Self::BYTES];
        out[0..8].copy_from_slice(&self.seed.to_le_bytes());
        out[8] = self.players;
        out[9] = self.grid;
        out[10] = self.props;
        out[11..15].copy_from_slice(&self.duration_frames.to_le_bytes());
        out[15..17].copy_from_slice(&self.score_limit.to_le_bytes());
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ConfigError> {
        let b: &[u8; Self::BYTES] = bytes
            .get(..Self::BYTES)
            .and_then(|b| b.try_into().ok())
            .ok_or(ConfigError::Truncated)?;
        let config = MatchConfig {
            seed: u64::from_le_bytes(b[0..8].try_into().expect("8 bytes")),
            players: b[8],
            grid: b[9],
            props: b[10],
            duration_frames: u32::from_le_bytes(b[11..15].try_into().expect("4 bytes")),
            score_limit: u16::from_le_bytes(b[15..17].try_into().expect("2 bytes")),
        };
        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip() {
        let config = MatchConfig {
            seed: 0x0123_4567_89ab_cdef,
            players: 3,
            grid: 11,
            props: 7,
            duration_frames: 123_456,
            score_limit: 9,
        };
        assert_eq!(MatchConfig::from_bytes(&config.to_bytes()), Ok(config));
    }

    #[test]
    fn rejects_bad_player_counts_and_short_input() {
        let config = MatchConfig {
            players: 5,
            ..MatchConfig::default()
        };
        assert_eq!(
            MatchConfig::from_bytes(&config.to_bytes()),
            Err(ConfigError::Players(5))
        );
        assert_eq!(
            MatchConfig::from_bytes(&[0; 5]),
            Err(ConfigError::Truncated)
        );
    }
}
