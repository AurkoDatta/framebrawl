//! Bots are input policies: they look at the world and produce the same
//! two-byte input a player would. They run wherever the authority runs and
//! their inputs travel the same path as everyone else's, so clients cannot
//! tell a bot's input from a human's and replays need nothing special.

use fb_sim::{Input, Pcg32, TileState, World, aim_towards};

const SHOVE_RANGE: f32 = 1.7;
/// How far ahead a bot checks the floor before committing to a direction.
const LOOKAHEAD: f32 = 1.6;
const DASH_NEAR: f32 = 3.0;
const DASH_FAR: f32 = 6.5;

pub struct Bot {
    rng: Pcg32,
    held: Input,
    /// Frames until the next decision. Bots react on a human timescale
    /// rather than every frame.
    rethink_in: u32,
}

impl Bot {
    pub fn new(seed: u64) -> Self {
        Bot {
            rng: Pcg32::new(seed),
            held: Input::default(),
            rethink_in: 0,
        }
    }

    /// The input for player `me` on the world's current frame.
    pub fn decide(&mut self, world: &World, me: usize) -> Input {
        if self.rethink_in > 0 {
            self.rethink_in -= 1;
            // Jump, shove and dash fire on the press, so they are taps:
            // held for the decision frame only.
            self.held.buttons &= Input::MOVE;
            return self.held;
        }
        self.rethink_in = 3 + self.rng.below(6);
        self.held = self.think(world, me);
        self.held
    }

    fn think(&mut self, world: &World, me: usize) -> Input {
        let view = world.player(me);
        if !view.in_play {
            return Input::default();
        }
        let (x, z) = (view.position[0], view.position[2]);
        let solid = |x: f32, z: f32| {
            world
                .tile_under(x, z)
                .is_some_and(|tile| world.tile(tile).0 == TileState::Solid)
        };

        // Standing on something that is about to go: leave first.
        if !solid(x, z) {
            return match nearest_solid_tile(world, x, z) {
                Some((tx, tz)) => {
                    let mut buttons = Input::MOVE;
                    if view.grounded && self.rng.below(3) == 0 {
                        buttons |= Input::JUMP;
                    }
                    Input::new(buttons, aim_towards(tx - x, tz - z))
                }
                None => Input::default(),
            };
        }

        let target = (0..world.player_count())
            .filter(|&other| other != me)
            .map(|other| world.player(other))
            .filter(|other| other.in_play && !other.shielded)
            .map(|other| (other.position[0] - x, other.position[2] - z))
            .min_by(|a, b| (a.0 * a.0 + a.1 * a.1).total_cmp(&(b.0 * b.0 + b.1 * b.1)));
        let Some((dx, dz)) = target else {
            // Nobody to fight: drift towards the middle.
            let aim = aim_towards(-x, -z);
            let moving = x * x + z * z > 4.0;
            return Input::new(if moving { Input::MOVE } else { 0 }, aim);
        };

        let distance = (dx * dx + dz * dz).sqrt();
        let aim = aim_towards(dx, dz);
        if distance < SHOVE_RANGE {
            let buttons = if view.shove_cooldown == 0 {
                Input::MOVE | Input::SHOVE
            } else {
                Input::MOVE
            };
            return Input::new(buttons, aim);
        }

        let (nx, nz) = (dx / distance, dz / distance);
        if !solid(x + nx * LOOKAHEAD, z + nz * LOOKAHEAD) {
            // The straight line crosses a hole or the edge. Go round by
            // way of the nearest safe tile in roughly that direction.
            let detour = nearest_solid_tile(world, x + nx * LOOKAHEAD, z + nz * LOOKAHEAD);
            return match detour {
                Some((tx, tz)) if (tx - x).abs() + (tz - z).abs() > 0.5 => {
                    Input::new(Input::MOVE, aim_towards(tx - x, tz - z))
                }
                _ => Input::new(0, aim),
            };
        }

        let mut buttons = Input::MOVE;
        let dash_is_safe = solid(x + nx * DASH_NEAR, z + nz * DASH_NEAR);
        if (DASH_NEAR..DASH_FAR).contains(&distance)
            && view.dash_cooldown == 0
            && dash_is_safe
            && self.rng.below(4) == 0
        {
            buttons |= Input::DASH;
        }
        Input::new(buttons, aim)
    }
}

fn nearest_solid_tile(world: &World, x: f32, z: f32) -> Option<(f32, f32)> {
    (0..world.tile_count())
        .filter(|&tile| world.tile(tile).0 == TileState::Solid)
        .map(|tile| world.tile_center(tile))
        .min_by(|a, b| {
            let da = (a.0 - x) * (a.0 - x) + (a.1 - z) * (a.1 - z);
            let db = (b.0 - x) * (b.0 - x) + (b.1 - z) * (b.1 - z);
            da.total_cmp(&db)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fb_sim::MatchConfig;

    fn play(seed: u64, frames: u32) -> World {
        let config = MatchConfig {
            seed,
            duration_frames: u32::MAX,
            score_limit: u16::MAX,
            ..MatchConfig::default()
        };
        let mut world = World::new(config);
        let mut bots: Vec<Bot> = (0..4).map(|i| Bot::new(seed * 10 + i)).collect();
        for _ in 0..frames {
            let inputs: Vec<Input> = bots
                .iter_mut()
                .enumerate()
                .map(|(me, bot)| bot.decide(&world, me))
                .collect();
            world.step(&inputs);
        }
        world
    }

    #[test]
    fn bots_fight_and_score() {
        for seed in 1..=5 {
            let world = play(seed, 60 * 60);
            let scores: u16 = (0..4).map(|i| world.player(i).score).sum();
            assert!(scores >= 3, "seed {seed}: {scores} ring-outs in a minute");
        }
    }

    #[test]
    fn bots_rarely_fall_off_on_their_own() {
        // Before tiles start cracking the only way out is over the edge.
        // Nearly every fall should be somebody's doing.
        let (mut falls, mut scores) = (0u32, 0u32);
        for seed in 1..=10 {
            let world = play(seed, 590);
            for i in 0..4 {
                falls += world.player(i).falls as u32;
                scores += world.player(i).score as u32;
            }
        }
        assert!(
            falls > 0,
            "ten seconds of four bots should produce a ring-out"
        );
        assert!(
            falls - scores <= falls / 5,
            "{falls} falls but only {scores} were ring-outs"
        );
    }

    #[test]
    fn bots_keep_playing_as_the_arena_shrinks() {
        let world = play(7, 60 * 150);
        let solid = (0..world.tile_count())
            .filter(|&t| world.tile(t).0 == TileState::Solid)
            .count();
        assert!(solid < 20, "{solid} tiles left after 150 seconds");
        let scores: u16 = (0..4).map(|i| world.player(i).score).sum();
        assert!(scores >= 10, "{scores} ring-outs over the match");
    }

    #[test]
    fn the_same_seed_makes_the_same_decisions() {
        assert_eq!(play(3, 900).checksum(), play(3, 900).checksum());
        assert_ne!(play(3, 900).checksum(), play(4, 900).checksum());
    }
}
