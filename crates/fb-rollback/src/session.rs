use fb_sim::Input;
use fb_sim::canon::Field;

use crate::frame_log::FrameLog;
use crate::pacing::{DelayPolicy, Pacer};
use crate::protocol::{ClientPacket, MAX_FRAMES_PER_PACKET, ServerPacket};
use crate::ring::StateRing;
use crate::{Frame, Simulation};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputDelay {
    Fixed(u8),
    /// Chosen from measured prediction depth, between 0 and `max`.
    Adaptive {
        max: u8,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct SessionConfig {
    /// Most frames the client will simulate past the last confirmed one
    /// before it stops and waits for the server.
    pub max_prediction: Frame,
    pub input_delay: InputDelay,
    /// Must match the server's checkpoint interval.
    pub checkpoint_interval: Frame,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            max_prediction: 30,
            input_delay: InputDelay::Adaptive { max: 3 },
            checkpoint_interval: 30,
        }
    }
}

/// A correction: the client rewound to `from` and simulated `depth` frames
/// again to get back to the present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rollback {
    pub from: Frame,
    pub depth: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Desync {
    pub frame: Frame,
    pub ours: u64,
    pub theirs: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub rollbacks: u32,
    pub resimulated_frames: u32,
    pub deepest_rollback: u32,
    /// Ticks on which the client refused to predict any further.
    pub stalls: u32,
    /// Confirmed frames simulated because the client had fallen behind.
    pub catch_up_frames: u32,
    /// Local inputs the server replaced because they arrived late.
    pub substituted_inputs: u32,
    pub checkpoints_verified: u32,
}

/// The client side: simulates ahead of the server on predicted inputs and
/// rewinds when the server's input stream says a prediction was wrong.
pub struct Session<S> {
    config: SessionConfig,
    players: usize,
    local: usize,

    /// The predicted present: state at `frame`.
    state: S,
    frame: Frame,
    /// First frame whose authoritative inputs have not arrived.
    confirmed: Frame,

    /// State at the start of each recent frame.
    ring: StateRing<S>,
    /// Inputs each frame in `confirmed..frame` was last simulated with.
    used: FrameLog<Vec<Input>>,
    /// Server inputs received during the current `receive` call.
    authoritative: FrameLog<Vec<Input>>,
    last_authoritative: Vec<Input>,
    /// Local inputs by the frame they are scheduled for.
    local_inputs: FrameLog<Input>,

    delay: u8,
    delay_policy: Option<DelayPolicy>,
    pacer: Pacer,
    pending_checkpoint: Option<(Frame, u64)>,
    /// Newest checkpoint frame already compared; packets repeat it.
    last_checkpoint: Frame,
    desync: Option<Desync>,
    stats: Stats,
}

impl<S: Simulation> Session<S> {
    pub fn new(sim: S, players: usize, local: usize, config: SessionConfig) -> Self {
        assert!(local < players);
        let (delay, delay_policy) = match config.input_delay {
            InputDelay::Fixed(delay) => (delay, None),
            InputDelay::Adaptive { max } => (0, Some(DelayPolicy::new(max))),
        };
        Session {
            config,
            players,
            local,
            state: sim,
            frame: 0,
            confirmed: 0,
            ring: StateRing::new((config.max_prediction + config.checkpoint_interval + 2) as usize),
            used: FrameLog::starting_at(0),
            authoritative: FrameLog::starting_at(0),
            last_authoritative: vec![Input::default(); players],
            local_inputs: FrameLog::starting_at(0),
            delay,
            delay_policy,
            pacer: Pacer::default(),
            pending_checkpoint: None,
            last_checkpoint: 0,
            desync: None,
            stats: Stats::default(),
        }
    }

    pub fn state(&self) -> &S {
        &self.state
    }

    /// The frame the predicted state is at.
    pub fn frame(&self) -> Frame {
        self.frame
    }

    pub fn confirmed_frame(&self) -> Frame {
        self.confirmed
    }

    pub fn input_delay(&self) -> u8 {
        self.delay
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn desync(&self) -> Option<Desync> {
        self.desync
    }

    /// How many times to call [`advance`](Self::advance) on this 60 Hz
    /// timer tick to stay the right distance ahead of the server.
    pub fn frames_due(&mut self) -> u32 {
        // A catch-up burst may not run past the prediction limit; one frame
        // is always offered so a stalled client keeps counting stalls.
        let room = self.config.max_prediction - (self.frame - self.confirmed);
        self.pacer.frames_due().min(room.max(1))
    }

    /// Simulates one frame of prediction with `local_input` as the input
    /// sampled now. Returns false, without simulating, if the client is
    /// already at its prediction limit.
    pub fn advance(&mut self, local_input: Input) -> bool {
        if self.frame - self.confirmed >= self.config.max_prediction {
            self.stats.stalls += 1;
            return false;
        }

        // The sample takes effect `delay` frames from now. If the delay
        // just grew, the frames in between repeat this sample; if it just
        // shrank, the slot is already taken and the sample is dropped, so
        // inputs already sent to the server are never rewritten.
        if self.local_inputs.end() < self.frame {
            self.local_inputs.trim_before(self.frame);
        }
        while self.local_inputs.end() <= self.frame + self.delay as Frame {
            self.local_inputs.push(local_input);
        }

        let inputs = self.inputs_for(self.frame);
        self.ring.save(self.frame, &self.state);
        self.state.step(&inputs);
        self.used.push(inputs);
        self.frame += 1;

        if let Some(policy) = &mut self.delay_policy {
            self.delay = policy.update(self.frame - self.confirmed, self.delay);
        }
        true
    }

    /// Best current knowledge of everyone's input for `frame`: the server's
    /// if it has arrived, otherwise our own scheduled input and each remote
    /// player's last confirmed input repeated.
    fn inputs_for(&self, frame: Frame) -> Vec<Input> {
        if let Some(row) = self.authoritative.get(frame) {
            return row.clone();
        }
        let mut row = self.last_authoritative.clone();
        row[self.local] = match self.local_inputs.get(frame) {
            Some(&input) => input,
            None if frame >= self.local_inputs.end() => self
                .local_inputs
                .end()
                .checked_sub(1)
                .and_then(|last| self.local_inputs.get(last))
                .copied()
                .unwrap_or_default(),
            // Before the first scheduled input: the opening delay frames.
            None => Input::default(),
        };
        row
    }

    /// Applies a server packet. Returns the rollback it caused, if any.
    pub fn receive(&mut self, packet: &ServerPacket) -> Option<Rollback> {
        self.pacer.sample(packet.lead);
        if let Some(checkpoint) = packet.checkpoint
            && checkpoint.0 > self.last_checkpoint
            && self
                .pending_checkpoint
                .is_none_or(|held| checkpoint.0 > held.0)
        {
            self.pending_checkpoint = Some(checkpoint);
        }

        let frames = packet.frames(self.players) as Frame;
        let packet_end = packet.first_frame + frames;
        // A packet that starts after what we have leaves a hole; the server
        // will resend from our acknowledgement. One that ends before it is
        // a duplicate or arrived out of order.
        if packet.first_frame > self.confirmed || packet_end <= self.confirmed {
            self.verify_checkpoint();
            return None;
        }

        let newly_confirmed = self.confirmed..packet_end;
        for frame in newly_confirmed.clone() {
            let row = packet.row(self.players, (frame - packet.first_frame) as usize);
            if self
                .local_inputs
                .get(frame)
                .is_some_and(|sent| *sent != row[self.local])
            {
                self.stats.substituted_inputs += 1;
            }
            self.authoritative.push(row.to_vec());
        }
        self.confirmed = packet_end;
        self.last_authoritative = self
            .authoritative
            .get(packet_end - 1)
            .expect("just pushed")
            .clone();

        // Predicted frames are checked too: a new last-confirmed input
        // changes what the frames after it should have assumed.
        let mispredicted = (newly_confirmed.start..self.frame)
            .find(|&frame| self.used.get(frame) != Some(&self.inputs_for(frame)));
        let rollback = mispredicted.map(|from| self.resimulate_from(from));

        // Fell behind the server (a stall, or a long gap): run the confirmed
        // frames we never predicted.
        while self.frame < self.confirmed {
            let inputs = self.inputs_for(self.frame);
            self.ring.save(self.frame, &self.state);
            self.state.step(&inputs);
            self.used.push(inputs);
            self.frame += 1;
            self.stats.catch_up_frames += 1;
        }

        self.verify_checkpoint();
        self.authoritative.trim_before(self.confirmed);
        self.used.trim_before(self.confirmed);
        self.local_inputs.trim_before(self.confirmed);
        rollback
    }

    fn resimulate_from(&mut self, from: Frame) -> Rollback {
        let saved = self
            .ring
            .get(from)
            .expect("ring covers the prediction window");
        self.state.clone_from(saved);
        for frame in from..self.frame {
            let inputs = self.inputs_for(frame);
            self.ring.save(frame, &self.state);
            self.state.step(&inputs);
            *self.used.get_mut(frame).expect("frame was simulated") = inputs;
        }
        let depth = self.frame - from;
        self.stats.rollbacks += 1;
        self.stats.resimulated_frames += depth;
        self.stats.deepest_rollback = self.stats.deepest_rollback.max(depth);
        Rollback { from, depth }
    }

    fn state_at(&self, frame: Frame) -> Option<&S> {
        if frame == self.frame {
            Some(&self.state)
        } else {
            self.ring.get(frame)
        }
    }

    /// Compares our state with the server's at its checkpoint, once every
    /// input before that frame is confirmed.
    fn verify_checkpoint(&mut self) {
        let Some((frame, theirs)) = self.pending_checkpoint else {
            return;
        };
        if frame > self.confirmed {
            return;
        }
        self.pending_checkpoint = None;
        self.last_checkpoint = frame;
        if let Some(state) = self.state_at(frame) {
            let ours = state.checksum();
            self.stats.checkpoints_verified += 1;
            if ours != theirs && self.desync.is_none() {
                self.desync = Some(Desync {
                    frame,
                    ours,
                    theirs,
                });
            }
        }
    }

    /// The packet to send this tick: our acknowledgement and every local
    /// input the server has not confirmed, so a lost packet costs nothing
    /// as long as a later one arrives in time.
    pub fn outgoing(&self) -> ClientPacket {
        let end = self.local_inputs.end();
        let first_frame = self
            .confirmed
            .max(self.local_inputs.start())
            .max(end.saturating_sub(MAX_FRAMES_PER_PACKET as Frame))
            .min(end);
        ClientPacket {
            ack: self.confirmed,
            first_frame,
            inputs: (first_frame..end)
                .filter_map(|frame| self.local_inputs.get(frame).copied())
                .collect(),
        }
    }

    /// Replaces the whole state with a server snapshot taken at `frame`.
    /// `last_inputs` are the inputs the server used on the frame before,
    /// which seed the prediction of remote players.
    pub fn resync(&mut self, frame: Frame, state: S, last_inputs: &[Input]) {
        assert_eq!(last_inputs.len(), self.players);
        self.state = state;
        self.frame = frame;
        self.confirmed = frame;
        self.ring.clear();
        self.used.reset(frame);
        self.authoritative.reset(frame);
        self.local_inputs.reset(frame);
        self.last_authoritative = last_inputs.to_vec();
        self.pending_checkpoint = None;
        self.last_checkpoint = frame;
        self.desync = None;
        self.pacer.reset();
    }

    /// Checksums of the confirmed states still held, for bisecting a desync
    /// against the server's list.
    pub fn recent_checksums(&self) -> Vec<(Frame, u64)> {
        let oldest = self
            .confirmed
            .saturating_sub(self.config.checkpoint_interval + 1);
        (oldest..=self.confirmed)
            .filter_map(|frame| Some((frame, self.state_at(frame)?.checksum())))
            .collect()
    }

    pub fn dump(&self, frame: Frame) -> Option<Vec<Field>> {
        self.state_at(frame).map(Simulation::fields)
    }
}
