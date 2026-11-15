//! Server-authoritative rollback.
//!
//! The server ([`Authority`]) owns the one true input stream: it assigns
//! each player's input to a frame, substitutes the previous input when one
//! is late, and never changes its mind. Each client ([`Session`]) runs
//! ahead of the server on predicted inputs, and when the server's stream
//! disagrees with a prediction it restores the state from before the wrong
//! frame and simulates forward again.
//!
//! Nothing here touches a clock or a socket. Time is counted in frames and
//! packets are plain structs, so the same code runs on the server, in the
//! browser, and in tests over a simulated bad network.

mod authority;
pub mod bisect;
mod frame_log;
mod pacing;
mod protocol;
mod ring;
mod session;
mod sim;

pub use authority::Authority;
pub use frame_log::FrameLog;
pub use pacing::{DelayPolicy, Pacer};
pub use protocol::{ClientPacket, MAX_FRAMES_PER_PACKET, ServerPacket};
pub use ring::StateRing;
pub use session::{Desync, InputDelay, Rollback, Session, SessionConfig, Stats};
pub use sim::Simulation;

pub type Frame = u32;
