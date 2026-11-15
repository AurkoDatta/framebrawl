//! Seeded random-input runs, shared by the native tests and the wasm
//! harness so every target plays exactly the same match.

use crate::{Input, MatchConfig, Pcg32, World, aim_towards};

/// Crude brawlers: each player holds a choice for a random stretch of
/// frames, mostly charging the nearest opponent and swinging, sometimes
/// heading for the centre, sometimes doing something random. Enough to
/// produce hits, ring-outs and respawns without a real bot.
pub struct InputScript {
    rng: Pcg32,
    held: Vec<(Input, u32)>,
}

impl InputScript {
    pub fn new(seed: u64, players: usize) -> Self {
        InputScript {
            rng: Pcg32::new(seed ^ 0x1bad_5eed_0ddb_a115),
            held: vec![(Input::default(), 0); players],
        }
    }

    pub fn next_frame(&mut self, world: &World) -> Vec<Input> {
        for me in 0..self.held.len() {
            if self.held[me].1 == 0 {
                self.held[me] = (self.choose(world, me), 1 + self.rng.below(24));
            }
            self.held[me].1 -= 1;
        }
        self.held.iter().map(|(input, _)| *input).collect()
    }

    fn choose(&mut self, world: &World, me: usize) -> Input {
        let rng = &mut self.rng;
        let at = world.player(me).position;
        let mut buttons = 0;
        if rng.below(12) == 0 {
            buttons |= Input::JUMP;
        }
        if rng.below(12) == 0 {
            buttons |= Input::DASH;
        }
        match rng.below(10) {
            0..=5 => {
                let nearest = (0..world.player_count())
                    .filter(|&other| other != me && world.player(other).in_play)
                    .map(|other| {
                        let p = world.player(other).position;
                        (p[0] - at[0], p[2] - at[2])
                    })
                    .min_by(|a, b| {
                        let (da, db) = (a.0 * a.0 + a.1 * a.1, b.0 * b.0 + b.1 * b.1);
                        da.total_cmp(&db)
                    });
                let (dx, dz) = nearest.unwrap_or((-at[0], -at[2]));
                if dx * dx + dz * dz < 4.0 || rng.below(3) == 0 {
                    buttons |= Input::SHOVE;
                }
                Input::new(buttons | Input::MOVE, aim_towards(dx, dz))
            }
            6..=7 => Input::new(buttons | Input::MOVE, aim_towards(-at[0], -at[2])),
            8 => Input::new(buttons, rng.below(256) as u8),
            _ => Input::new(buttons | Input::MOVE, rng.below(256) as u8),
        }
    }
}

/// Checksums after each of `frames` steps.
pub fn checksums(config: MatchConfig, frames: u32) -> Vec<u64> {
    let mut world = World::new(config);
    let mut script = InputScript::new(config.seed, config.players as usize);
    (0..frames)
        .map(|_| {
            let inputs = script.next_frame(&world);
            world.step(&inputs);
            world.checksum()
        })
        .collect()
}

/// The same run as [`checksums`], but rewinding up to `max_depth` frames at
/// random points and resimulating from a snapshot. Returns the checksum
/// produced the last time each frame was simulated.
pub fn checksums_with_rewinds(config: MatchConfig, frames: u32, max_depth: u32) -> Vec<u64> {
    let mut world = World::new(config);
    let mut script = InputScript::new(config.seed, config.players as usize);
    let mut schedule = Pcg32::new(config.seed ^ 0x7e57_ab1e);
    let mut inputs: Vec<Vec<Input>> = Vec::new();
    let mut snapshots: Vec<Option<World>> = Vec::new();
    let mut out: Vec<u64> = Vec::new();

    for frame in 0..frames as usize {
        inputs.push(script.next_frame(&world));
        snapshots.push(Some(world.clone()));
        if let Some(stale) = frame.checked_sub(max_depth as usize + 1) {
            snapshots[stale] = None;
        }
        world.step(&inputs[frame]);
        out.push(world.checksum());

        if schedule.below(4) == 0 {
            let depth = (1 + schedule.below(max_depth) as usize).min(frame + 1);
            let from = frame + 1 - depth;
            world.clone_from(snapshots[from].as_ref().expect("snapshot within depth"));
            for f in from..=frame {
                world.step(&inputs[f]);
                out[f] = world.checksum();
            }
        }
    }
    out
}

/// FNV-1a 64 over a checksum stream, for comparing whole runs at a glance.
pub fn digest(stream: &[u64]) -> u64 {
    use crate::canon::{Fnv64, Sink};
    let mut hash = Fnv64::default();
    for &checksum in stream {
        hash.put("", 0, checksum);
    }
    hash.0
}
