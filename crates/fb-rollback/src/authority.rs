use std::collections::BTreeMap;

use fb_sim::Input;
use fb_sim::canon::Field;

use crate::frame_log::FrameLog;
use crate::protocol::{ClientPacket, MAX_FRAMES_PER_PACKET, ServerPacket};
use crate::ring::StateRing;
use crate::{Frame, Simulation};

/// Frames of confirmed input kept for resending to clients that fall behind.
const HISTORY: Frame = 256;
/// How far ahead of the server a client may schedule input.
const MAX_FUTURE: Frame = 120;

/// The server side: decides which input each player gave on each frame and
/// advances the one simulation everybody has to agree with.
pub struct Authority<S> {
    sim: S,
    frame: Frame,
    players: usize,
    /// Inputs received for frames not simulated yet.
    pending: Vec<BTreeMap<Frame, Input>>,
    last_input: Vec<Input>,
    newest: Vec<Option<Frame>>,
    acks: Vec<Frame>,
    lead: Vec<i8>,
    substitutions: Vec<u32>,
    history: FrameLog<Vec<Input>>,
    states: StateRing<S>,
    checkpoint_interval: Frame,
    checkpoint: Option<(Frame, u64)>,
}

impl<S: Simulation> Authority<S> {
    pub fn new(sim: S, players: usize, checkpoint_interval: Frame) -> Self {
        assert!(checkpoint_interval > 0);
        Authority {
            sim,
            frame: 0,
            players,
            pending: vec![BTreeMap::new(); players],
            last_input: vec![Input::default(); players],
            newest: vec![None; players],
            acks: vec![0; players],
            lead: vec![0; players],
            substitutions: vec![0; players],
            history: FrameLog::starting_at(0),
            states: StateRing::new(checkpoint_interval as usize + 2),
            checkpoint_interval,
            checkpoint: None,
        }
    }

    /// The next frame to be simulated.
    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn state(&self) -> &S {
        &self.sim
    }

    /// The inputs used on the most recent frame; what a freshly resynced
    /// client should assume remote players are still holding.
    pub fn last_inputs(&self) -> &[Input] {
        &self.last_input
    }

    /// Times the server had to invent an input for `player`.
    pub fn substitutions(&self, player: usize) -> u32 {
        self.substitutions[player]
    }

    /// Offers an input for a future frame. The first input offered for a
    /// frame wins; anything for a frame already simulated is too late.
    pub fn offer(&mut self, player: usize, frame: Frame, input: Input) {
        if frame < self.frame || frame >= self.frame + MAX_FUTURE {
            return;
        }
        self.pending[player].entry(frame).or_insert(input);
        if self.newest[player].is_none_or(|newest| frame > newest) {
            self.newest[player] = Some(frame);
        }
    }

    pub fn receive(&mut self, player: usize, packet: &ClientPacket) {
        // An acknowledgement of frames that do not exist yet is nonsense;
        // believing it would stop this client's packets for good.
        if packet.ack <= self.frame {
            self.acks[player] = self.acks[player].max(packet.ack);
        }
        for (offset, &input) in packet.inputs.iter().take(MAX_FRAMES_PER_PACKET).enumerate() {
            self.offer(player, packet.first_frame + offset as Frame, input);
        }
    }

    /// Simulates one frame and returns the inputs it used. A player whose
    /// input has not arrived repeats their previous one, and that decision
    /// is final.
    pub fn tick(&mut self) -> &[Input] {
        let mut inputs = Vec::with_capacity(self.players);
        for player in 0..self.players {
            let input = match self.pending[player].remove(&self.frame) {
                Some(input) => input,
                None => {
                    self.substitutions[player] += 1;
                    self.last_input[player]
                }
            };
            self.last_input[player] = input;
            inputs.push(input);
            self.lead[player] = match self.newest[player] {
                Some(newest) => (newest as i64 - self.frame as i64).clamp(-127, 127) as i8,
                // Nothing received yet says little about timing; call it
                // slightly late so the client leans forward.
                None => -1,
            };
        }

        self.states.save(self.frame, &self.sim);
        self.sim.step(&inputs);
        self.history.push(inputs);
        self.frame += 1;
        self.history.trim_before(self.frame.saturating_sub(HISTORY));
        if self.frame.is_multiple_of(self.checkpoint_interval) {
            self.checkpoint = Some((self.frame, self.sim.checksum()));
        }
        self.history.get(self.frame - 1).expect("just pushed")
    }

    /// Everything `player` has not acknowledged, oldest first, up to the
    /// packet limit. `None` if the client is so far behind that the history
    /// it needs is gone and it must be sent a snapshot instead.
    pub fn packet_for(&self, player: usize) -> Option<ServerPacket> {
        let first_frame = self.acks[player].min(self.frame);
        if first_frame < self.history.start() {
            return None;
        }
        let end = self.frame.min(first_frame + MAX_FRAMES_PER_PACKET as Frame);
        let mut inputs = Vec::with_capacity((end - first_frame) as usize * self.players);
        for frame in first_frame..end {
            inputs.extend_from_slice(self.history.get(frame).expect("within history"));
        }
        Some(ServerPacket {
            first_frame,
            inputs,
            lead: self.lead[player],
            checkpoint: self.checkpoint,
        })
    }

    /// Tells the authority a client was handed a snapshot at the current
    /// frame, so its acknowledgement starts from here.
    pub fn snapshot_sent(&mut self, player: usize) {
        self.acks[player] = self.frame;
    }

    /// Checksums of the states still held, for bisecting a desync.
    pub fn recent_checksums(&self) -> Vec<(Frame, u64)> {
        let oldest = self.frame.saturating_sub(self.states.capacity() as Frame);
        let mut out: Vec<(Frame, u64)> = (oldest..self.frame)
            .filter_map(|f| Some((f, self.states.get(f)?.checksum())))
            .collect();
        out.push((self.frame, self.sim.checksum()));
        out
    }

    pub fn dump(&self, frame: Frame) -> Option<Vec<Field>> {
        if frame == self.frame {
            return Some(self.sim.fields());
        }
        self.states.get(frame).map(Simulation::fields)
    }
}
