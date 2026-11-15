//! What a client and the server say to each other, before any encoding.
//!
//! Both directions are sent every tick over an unreliable link and carry
//! enough history that any single packet brings the other side up to date.

use fb_sim::Input;

use crate::Frame;

/// Most frames of input carried in one packet, in either direction.
pub const MAX_FRAMES_PER_PACKET: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientPacket {
    /// The first authoritative frame this client has not received yet.
    pub ack: Frame,
    /// Frame of `inputs[0]`; the rest follow consecutively.
    pub first_frame: Frame,
    /// This client's inputs that the server has not confirmed, oldest first.
    pub inputs: Vec<Input>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerPacket {
    /// Frame of the first row in `inputs`.
    pub first_frame: Frame,
    /// Authoritative inputs, one row of `players` entries per frame.
    pub inputs: Vec<Input>,
    /// How many frames early this client's newest input was when the server
    /// last ticked. Negative means the server had to substitute.
    pub lead: i8,
    /// State checksum at a recent checkpoint frame.
    pub checkpoint: Option<(Frame, u64)>,
}

impl ServerPacket {
    pub fn frames(&self, players: usize) -> usize {
        self.inputs.len() / players
    }

    pub fn row(&self, players: usize, index: usize) -> &[Input] {
        &self.inputs[index * players..(index + 1) * players]
    }
}
