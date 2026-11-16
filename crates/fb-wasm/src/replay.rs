//! Plays a recorded match back, checking it against its own checksums as
//! it goes.

use fb_replay::Replay;
use fb_sim::World;
use wasm_bindgen::prelude::*;

use crate::view::write_spectator_view;

#[wasm_bindgen]
pub struct ReplayPlayer {
    replay: Replay,
    world: World,
    /// Index of the next recorded checkpoint to compare.
    next_checkpoint: usize,
    matched: u32,
    /// The first checkpoint frame that did not match, if any.
    diverged_at: Option<u32>,
    view: Vec<f32>,
}

#[wasm_bindgen]
impl ReplayPlayer {
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8]) -> Result<ReplayPlayer, JsError> {
        let replay = Replay::decode(bytes).map_err(|error| JsError::new(&error.to_string()))?;
        let world = World::new(replay.config);
        Ok(ReplayPlayer {
            replay,
            world,
            next_checkpoint: 0,
            matched: 0,
            diverged_at: None,
            view: Vec::new(),
        })
    }

    /// Simulates one frame. Returns false once the recording has run out.
    pub fn step(&mut self) -> bool {
        let frame = self.world.frame();
        if frame >= self.replay.frames() {
            return false;
        }
        self.world.step(self.replay.row(frame));
        if let Some(&(at, recorded)) = self.replay.checkpoints.get(self.next_checkpoint)
            && at == self.world.frame()
        {
            self.next_checkpoint += 1;
            if self.world.checksum() == recorded {
                self.matched += 1;
            } else if self.diverged_at.is_none() {
                self.diverged_at = Some(at);
            }
        }
        true
    }

    pub fn frame(&self) -> u32 {
        self.world.frame()
    }

    pub fn frames(&self) -> u32 {
        self.replay.frames()
    }

    pub fn players(&self) -> u8 {
        self.replay.config.players
    }

    pub fn duration_frames(&self) -> u32 {
        self.replay.config.duration_frames
    }

    pub fn score_limit(&self) -> u16 {
        self.replay.config.score_limit
    }

    pub fn checkpoints(&self) -> u32 {
        self.replay.checkpoints.len() as u32
    }

    /// The first checkpoint frame where this machine disagreed with the
    /// recording, or -1 if it has agreed so far.
    pub fn diverged_at(&self) -> i32 {
        self.diverged_at.map_or(-1, |frame| frame as i32)
    }

    pub fn view(&mut self) -> Vec<f32> {
        write_spectator_view(&self.world, self.matched, &mut self.view);
        self.view.clone()
    }
}
