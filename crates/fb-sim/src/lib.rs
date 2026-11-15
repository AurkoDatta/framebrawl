//! Deterministic simulation core.
//!
//! Everything here must produce identical bits on every target: no wall
//! clock, no OS randomness, no unordered iteration, no platform float
//! functions. The cross-target harness in `tests/e2e` checks that it does.

pub mod canon;
mod config;
mod heading;
mod input;
mod rng;
pub mod soak;
mod world;

pub use config::{ConfigError, MAX_PLAYERS, MatchConfig};
pub use heading::{aim_towards, heading};
pub use input::Input;
pub use rng::Pcg32;
pub use world::{BodyView, PlayerView, TICK_RATE, TileState, World};
