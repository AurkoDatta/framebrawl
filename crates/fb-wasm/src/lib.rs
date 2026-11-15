//! Browser bindings: the determinism harness entry points, the network
//! client the page runs in a worker, and the Netcode Lab.

mod client;
mod lab;
mod view;

pub use client::NetClient;
pub use lab::Lab;

use fb_sim::soak;
use fb_sim::{MatchConfig, World};
use wasm_bindgen::prelude::*;

fn soak_config(seed: u64, players: u8) -> MatchConfig {
    MatchConfig {
        seed,
        players,
        ..MatchConfig::default()
    }
}

/// Per-frame checksums of a seeded random-input match.
#[wasm_bindgen]
pub fn soak_checksums(seed: u64, players: u8, frames: u32) -> Vec<u64> {
    soak::checksums(soak_config(seed, players), frames)
}

/// The same match with random rewinds of up to `max_depth` frames.
#[wasm_bindgen]
pub fn soak_checksums_with_rewinds(
    seed: u64,
    players: u8,
    frames: u32,
    max_depth: u32,
) -> Vec<u64> {
    soak::checksums_with_rewinds(soak_config(seed, players), frames, max_depth)
}

/// Times the operations a rollback is made of. Loops run inside wasm so the
/// JS side can time many iterations with a coarse clock.
#[wasm_bindgen]
pub struct RollbackBench {
    world: World,
    snapshot: World,
    script: soak::InputScript,
}

#[wasm_bindgen]
impl RollbackBench {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u64, players: u8, warmup_frames: u32) -> RollbackBench {
        let config = soak_config(seed, players);
        let mut world = World::new(config);
        let mut script = soak::InputScript::new(seed, players as usize);
        for _ in 0..warmup_frames {
            let inputs = script.next_frame(&world);
            world.step(&inputs);
        }
        RollbackBench {
            snapshot: world.clone(),
            world,
            script,
        }
    }

    /// Restore, then `depth` times: save and step. Returns a checksum so
    /// the work cannot be optimised away.
    pub fn rollback(&mut self, iterations: u32, depth: u32) -> u64 {
        let mut sink = 0;
        for _ in 0..iterations {
            self.world.clone_from(&self.snapshot);
            for _ in 0..depth {
                let saved = self.world.clone();
                let inputs = self.script.next_frame(&self.world);
                self.world.step(&inputs);
                sink ^= saved.frame() as u64;
            }
            sink ^= self.world.checksum();
        }
        sink
    }

    pub fn step(&mut self, iterations: u32) {
        for _ in 0..iterations {
            let inputs = self.script.next_frame(&self.world);
            self.world.step(&inputs);
        }
    }

    pub fn save(&mut self, iterations: u32) {
        for _ in 0..iterations {
            self.snapshot = self.world.clone();
        }
    }
}
