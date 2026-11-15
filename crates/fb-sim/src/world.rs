use crate::canon::{Field, Fnv64, Sink};
use crate::config::MatchConfig;
use crate::heading::heading;
use crate::input::Input;
use crate::rng::Pcg32;
use rapier3d::prelude::*;

pub const TICK_RATE: u32 = 60;

const TILE_SIZE: f32 = 2.0;
const TILE_HALF_HEIGHT: f32 = 0.25;
const KILL_Y: f32 = -14.0;
const GRAVITY: f32 = -24.0;

const PLAYER_RADIUS: f32 = 0.4;
const PLAYER_HALF_HEIGHT: f32 = 0.35;
const RUN_SPEED: f32 = 7.0;
/// Largest horizontal velocity change per frame from steering.
const GROUND_ACCEL: f32 = 1.0;
const AIR_ACCEL: f32 = 0.3;
const JUMP_SPEED: f32 = 9.5;
const DASH_SPEED: f32 = 16.0;
const DASH_COOLDOWN: u8 = 50;

const SHOVE_REACH: f32 = 2.1;
/// Cosine of the half-angle of the shove cone (60 degrees either side).
const SHOVE_CONE: f32 = 0.5;
const SHOVE_COOLDOWN: u8 = 24;
const SHOVE_BASE: f32 = 7.0;
const SHOVE_PER_STRAIN: f32 = 0.11;
const SHOVE_LIFT: f32 = 4.5;
const STRAIN_PER_HIT: u16 = 12;
const STRAIN_MAX: u16 = 200;
const STUN_BASE: u8 = 10;
const PROP_SHOVE: f32 = 9.0;

const RESPAWN_FRAMES: u16 = 120;
const SHIELD_FRAMES: u16 = 90;
/// A ring-out counts for the last attacker if the hit was this recent.
const CREDIT_FRAMES: u32 = 360;
const NOBODY: u8 = u8::MAX;

const CRACK_GRACE: u32 = 600;
const CRACK_WARNING: u32 = 90;
const CRACK_INTERVAL_START: u32 = 150;
const CRACK_INTERVAL_MIN: u32 = 45;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileState {
    Solid,
    /// Visibly cracked; turns loose at `falls_at`.
    Cracking {
        falls_at: u32,
    },
    Falling,
    Gone,
}

#[derive(Clone, Debug)]
struct Tile {
    body: RigidBodyHandle,
    state: TileState,
}

#[derive(Clone, Debug)]
struct Player {
    body: RigidBodyHandle,
    collider: ColliderHandle,
    facing: u8,
    prev_buttons: u8,
    grounded: bool,
    strain: u16,
    stun: u8,
    shove_cooldown: u8,
    dash_cooldown: u8,
    /// Frames until respawn; zero while in play.
    respawn_in: u16,
    shield: u16,
    last_hit_by: u8,
    last_hit_frame: u32,
    score: u16,
    falls: u16,
}

/// Read-only view of a player for rendering and bots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerView {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub facing: u8,
    pub in_play: bool,
    pub grounded: bool,
    pub shielded: bool,
    pub stunned: bool,
    pub strain: u16,
    pub score: u16,
    pub falls: u16,
    pub shove_cooldown: u8,
    pub dash_cooldown: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyView {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
}

/// rapier's step workspace. It holds no state that outlives a step, so a
/// cloned world starts with a fresh one.
#[derive(Default)]
struct Workspace(PhysicsPipeline);

impl Clone for Workspace {
    fn clone(&self) -> Self {
        Workspace::default()
    }

    fn clone_from(&mut self, _source: &Self) {}
}

/// The complete simulation state. `Clone` is the rollback snapshot.
#[derive(Clone)]
pub struct World {
    config: MatchConfig,
    frame: u32,
    rng: Pcg32,
    next_crack: u32,
    players: Vec<Player>,
    tiles: Vec<Tile>,
    props: Vec<RigidBodyHandle>,

    // Everything rapier carries between steps. Contact caches and sleep
    // state feed the next step, so restoring bodies alone would diverge.
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    soft_bodies: SoftBodySet,
    ccd: CCDSolver,
    workspace: Workspace,
}

fn tile_center(grid: u8, index: usize) -> (f32, f32) {
    let g = grid as usize;
    let origin = -(grid as f32) * TILE_SIZE / 2.0 + TILE_SIZE / 2.0;
    (
        origin + (index % g) as f32 * TILE_SIZE,
        origin + (index / g) as f32 * TILE_SIZE,
    )
}

impl World {
    /// Panics if `config` fails [`MatchConfig::validate`].
    pub fn new(config: MatchConfig) -> Self {
        config.validate().expect("valid match config");
        let mut rng = Pcg32::new(config.seed);
        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();
        let grid = config.grid as usize;

        let mut tiles = Vec::with_capacity(grid * grid);
        for index in 0..grid * grid {
            let (x, z) = tile_center(config.grid, index);
            let body = bodies.insert(RigidBodyBuilder::fixed().translation(Vector::new(
                x,
                -TILE_HALF_HEIGHT,
                z,
            )));
            colliders.insert_with_parent(
                ColliderBuilder::cuboid(TILE_SIZE / 2.0, TILE_HALF_HEIGHT, TILE_SIZE / 2.0),
                body,
                &mut bodies,
            );
            tiles.push(Tile {
                body,
                state: TileState::Solid,
            });
        }

        // One tile in from each corner, facing the centre.
        let far = grid - 2;
        let spawns = [(1, 1, 32u8), (far, far, 160), (far, 1, 96), (1, far, 224)];
        let mut players = Vec::with_capacity(config.players as usize);
        for &(col, row, facing) in spawns.iter().take(config.players as usize) {
            let (x, z) = tile_center(config.grid, row * grid + col);
            let body = bodies.insert(
                RigidBodyBuilder::dynamic()
                    .translation(Vector::new(x, 1.0, z))
                    .lock_rotations()
                    .can_sleep(false),
            );
            let collider = colliders.insert_with_parent(
                ColliderBuilder::capsule_y(PLAYER_HALF_HEIGHT, PLAYER_RADIUS)
                    // Steering sets velocity directly; friction would only
                    // fight it and make speed depend on the surface.
                    .friction(0.0)
                    .friction_combine_rule(CoefficientCombineRule::Min)
                    .density(1.2),
                body,
                &mut bodies,
            );
            players.push(Player {
                body,
                collider,
                facing,
                prev_buttons: 0,
                grounded: false,
                strain: 0,
                stun: 0,
                shove_cooldown: 0,
                dash_cooldown: 0,
                respawn_in: 0,
                shield: SHIELD_FRAMES,
                last_hit_by: NOBODY,
                last_hit_frame: 0,
                score: 0,
                falls: 0,
            });
        }

        let mut props = Vec::with_capacity(config.props as usize);
        for i in 0..config.props as usize {
            let (x, z) = tile_center(config.grid, rng.below((grid * grid) as u32) as usize);
            let body = bodies.insert(RigidBodyBuilder::dynamic().translation(Vector::new(
                x,
                1.5 + i as f32 * 1.2,
                z,
            )));
            let shape = if i % 2 == 0 {
                ColliderBuilder::cuboid(0.4, 0.4, 0.4)
            } else {
                ColliderBuilder::ball(0.45)
            };
            colliders.insert_with_parent(shape.restitution(0.3).density(0.6), body, &mut bodies);
            props.push(body);
        }

        World {
            config,
            frame: 0,
            rng,
            next_crack: CRACK_GRACE,
            players,
            tiles,
            props,
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies,
            colliders,
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            soft_bodies: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            workspace: Workspace::default(),
        }
    }

    /// Advances one tick. `inputs` holds one entry per player, in player
    /// order.
    pub fn step(&mut self, inputs: &[Input]) {
        assert_eq!(inputs.len(), self.players.len(), "one input per player");
        self.steer(inputs);
        self.resolve_shoves(inputs);
        self.crack_tiles();
        self.step_physics();
        self.update_grounded();
        self.ring_outs_and_respawns();
        self.remove_fallen();
        for (player, input) in self.players.iter_mut().zip(inputs) {
            player.prev_buttons = input.buttons;
        }
        self.frame += 1;
    }

    fn steer(&mut self, inputs: &[Input]) {
        for (player, input) in self.players.iter_mut().zip(inputs) {
            player.shove_cooldown = player.shove_cooldown.saturating_sub(1);
            player.dash_cooldown = player.dash_cooldown.saturating_sub(1);
            player.shield = player.shield.saturating_sub(1);
            if player.respawn_in > 0 {
                continue;
            }
            if player.stun > 0 {
                player.stun -= 1;
                continue;
            }
            let pressed = input.buttons & !player.prev_buttons;
            if input.buttons != 0 {
                player.facing = input.aim;
            }
            let (hx, hz) = heading(player.facing);
            let body = &mut self.bodies[player.body];
            let mut v = body.linvel();

            let (want_x, want_z) = if input.held(Input::MOVE) {
                (hx * RUN_SPEED, hz * RUN_SPEED)
            } else {
                (0.0, 0.0)
            };
            let (dx, dz) = (want_x - v.x, want_z - v.z);
            let gap = (dx * dx + dz * dz).sqrt();
            let limit = if player.grounded {
                GROUND_ACCEL
            } else {
                AIR_ACCEL
            };
            if gap <= limit {
                v.x = want_x;
                v.z = want_z;
            } else {
                v.x += dx / gap * limit;
                v.z += dz / gap * limit;
            }

            if pressed & Input::DASH != 0 && player.dash_cooldown == 0 {
                player.dash_cooldown = DASH_COOLDOWN;
                v.x = hx * DASH_SPEED;
                v.z = hz * DASH_SPEED;
            }
            if pressed & Input::JUMP != 0 && player.grounded {
                player.grounded = false;
                v.y = JUMP_SPEED;
            }
            body.set_linvel(v, true);
        }
    }

    fn resolve_shoves(&mut self, inputs: &[Input]) {
        for (attacker, input) in inputs.iter().enumerate() {
            let a = &self.players[attacker];
            let pressed = input.buttons & !a.prev_buttons;
            if pressed & Input::SHOVE == 0 || a.shove_cooldown > 0 || a.respawn_in > 0 || a.stun > 0
            {
                continue;
            }
            let origin = self.bodies[a.body].translation();
            let (hx, hz) = heading(a.facing);
            self.players[attacker].shove_cooldown = SHOVE_COOLDOWN;

            // Direction of a hit on something at `target`, if it is in reach
            // and inside the cone.
            let aim_at = |target: Vector| -> Option<(f32, f32)> {
                let (dx, dy, dz) = (
                    target.x - origin.x,
                    target.y - origin.y,
                    target.z - origin.z,
                );
                if dx * dx + dy * dy + dz * dz > SHOVE_REACH * SHOVE_REACH {
                    return None;
                }
                let flat = (dx * dx + dz * dz).sqrt();
                if flat < 1e-4 {
                    return Some((hx, hz));
                }
                let (nx, nz) = (dx / flat, dz / flat);
                (nx * hx + nz * hz >= SHOVE_CONE).then_some((nx, nz))
            };

            for victim in 0..self.players.len() {
                let v = &self.players[victim];
                if victim == attacker || v.respawn_in > 0 || v.shield > 0 {
                    continue;
                }
                let Some((nx, nz)) = aim_at(self.bodies[v.body].translation()) else {
                    continue;
                };
                let v = &mut self.players[victim];
                v.strain = (v.strain + STRAIN_PER_HIT).min(STRAIN_MAX);
                v.stun = STUN_BASE + (v.strain / 16) as u8;
                v.last_hit_by = attacker as u8;
                v.last_hit_frame = self.frame;
                let force = SHOVE_BASE + v.strain as f32 * SHOVE_PER_STRAIN;
                let body = &mut self.bodies[v.body];
                let mut vel = body.linvel();
                vel.x = nx * force;
                vel.z = nz * force;
                vel.y = SHOVE_LIFT;
                body.set_linvel(vel, true);
            }

            for &prop in &self.props {
                let body = &mut self.bodies[prop];
                if let Some((nx, nz)) = aim_at(body.translation()) {
                    body.set_linvel(
                        Vector::new(nx * PROP_SHOVE, SHOVE_LIFT, nz * PROP_SHOVE),
                        true,
                    );
                }
            }
        }
    }

    fn crack_tiles(&mut self) {
        if self.frame >= self.next_crack {
            let elapsed = self.frame - CRACK_GRACE;
            self.next_crack = self.frame
                + CRACK_INTERVAL_START
                    .saturating_sub(elapsed / 60)
                    .max(CRACK_INTERVAL_MIN);
            let solid = self
                .tiles
                .iter()
                .filter(|t| t.state == TileState::Solid)
                .count();
            // Always leave somewhere to stand and respawn.
            if solid > self.players.len() {
                let pick = self.rng.below(solid as u32) as usize;
                let tile = self
                    .tiles
                    .iter_mut()
                    .filter(|t| t.state == TileState::Solid)
                    .nth(pick)
                    .expect("pick is below the solid count");
                tile.state = TileState::Cracking {
                    falls_at: self.frame + CRACK_WARNING,
                };
            }
        }

        for tile in &mut self.tiles {
            if let TileState::Cracking { falls_at } = tile.state
                && self.frame >= falls_at
            {
                tile.state = TileState::Falling;
                self.bodies[tile.body].set_body_type(RigidBodyType::Dynamic, true);
            }
        }
    }

    fn step_physics(&mut self) {
        let params = IntegrationParameters {
            dt: 1.0 / TICK_RATE as f32,
            ..IntegrationParameters::default()
        };
        self.workspace.0.step(
            Vector::new(0.0, GRAVITY, 0.0),
            &params,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &(),
            &(),
        );
    }

    fn update_grounded(&mut self) {
        for player in &mut self.players {
            // A contact whose normal points up at the player means it is
            // standing on something. Normals point from collider1 to
            // collider2.
            player.grounded = self
                .narrow_phase
                .contact_pairs_with(player.collider)
                .filter(|pair| pair.has_any_active_contact())
                .any(|pair| {
                    let sign = if pair.collider1 == player.collider {
                        -1.0
                    } else {
                        1.0
                    };
                    pair.manifolds()
                        .iter()
                        .any(|m| !m.points.is_empty() && m.data.normal.y * sign > 0.7)
                });
        }
    }

    fn ring_outs_and_respawns(&mut self) {
        for index in 0..self.players.len() {
            let player = &self.players[index];
            if player.respawn_in > 0 {
                self.players[index].respawn_in -= 1;
                if self.players[index].respawn_in == 0 {
                    self.respawn(index);
                }
                continue;
            }
            if self.bodies[player.body].translation().y >= KILL_Y {
                continue;
            }

            let attacker = player.last_hit_by as usize;
            let credited =
                player.last_hit_by != NOBODY && self.frame - player.last_hit_frame <= CREDIT_FRAMES;
            if credited {
                self.players[attacker].score += 1;
            }
            let player = &mut self.players[index];
            player.falls += 1;
            player.respawn_in = RESPAWN_FRAMES;
            player.last_hit_by = NOBODY;
            player.strain = 0;
            player.stun = 0;
            player.grounded = false;
            self.bodies[player.body].set_enabled(false);
        }
    }

    fn respawn(&mut self, index: usize) {
        let standable = |t: &&Tile| t.state == TileState::Solid;
        let count = self.tiles.iter().filter(standable).count();
        let (x, z) = if count == 0 {
            (0.0, 0.0)
        } else {
            let pick = self.rng.below(count as u32) as usize;
            let tile = self
                .tiles
                .iter()
                .enumerate()
                .filter(|(_, t)| standable(t))
                .nth(pick)
                .expect("pick is below the solid count");
            tile_center(self.config.grid, tile.0)
        };
        let player = &mut self.players[index];
        player.shield = SHIELD_FRAMES;
        let body = &mut self.bodies[player.body];
        body.set_enabled(true);
        body.set_translation(Vector::new(x, 3.0, z), true);
        body.set_linvel(Vector::new(0.0, 0.0, 0.0), true);
    }

    fn remove_fallen(&mut self) {
        for tile in &mut self.tiles {
            if tile.state == TileState::Falling && self.bodies[tile.body].translation().y < KILL_Y {
                tile.state = TileState::Gone;
                self.bodies.remove(
                    tile.body,
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    &mut self.soft_bodies,
                    true,
                );
            }
        }
        let mut i = 0;
        while i < self.props.len() {
            if self.bodies[self.props[i]].translation().y < KILL_Y {
                self.bodies.remove(
                    self.props.remove(i),
                    &mut self.islands,
                    &mut self.colliders,
                    &mut self.impulse_joints,
                    &mut self.multibody_joints,
                    &mut self.soft_bodies,
                    true,
                );
            } else {
                i += 1;
            }
        }
    }

    pub fn config(&self) -> &MatchConfig {
        &self.config
    }

    pub fn frame(&self) -> u32 {
        self.frame
    }

    pub fn finished(&self) -> bool {
        self.frame >= self.config.duration_frames
            || self
                .players
                .iter()
                .any(|p| p.score >= self.config.score_limit)
    }

    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    pub fn player(&self, index: usize) -> PlayerView {
        let p = &self.players[index];
        let body = &self.bodies[p.body];
        PlayerView {
            position: body.translation().to_array(),
            velocity: body.linvel().to_array(),
            facing: p.facing,
            in_play: p.respawn_in == 0,
            grounded: p.grounded,
            shielded: p.shield > 0,
            stunned: p.stun > 0,
            strain: p.strain,
            score: p.score,
            falls: p.falls,
            shove_cooldown: p.shove_cooldown,
            dash_cooldown: p.dash_cooldown,
        }
    }

    /// Centre of tile `index` on the ground plane, as `(x, z)`.
    pub fn tile_center(&self, index: usize) -> (f32, f32) {
        tile_center(self.config.grid, index)
    }

    /// The tile under `(x, z)`, whatever its state, or `None` outside the
    /// arena footprint.
    pub fn tile_under(&self, x: f32, z: f32) -> Option<usize> {
        let grid = self.config.grid as f32;
        let half = grid * TILE_SIZE / 2.0;
        let (col, row) = ((x + half) / TILE_SIZE, (z + half) / TILE_SIZE);
        if col < 0.0 || row < 0.0 || col >= grid || row >= grid {
            return None;
        }
        Some(row as usize * self.config.grid as usize + col as usize)
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn tile(&self, index: usize) -> (TileState, Option<BodyView>) {
        let tile = &self.tiles[index];
        (tile.state, self.body_view(tile.body))
    }

    pub fn props(&self) -> impl Iterator<Item = BodyView> + '_ {
        self.props.iter().filter_map(|&h| self.body_view(h))
    }

    fn body_view(&self, handle: RigidBodyHandle) -> Option<BodyView> {
        self.bodies.get(handle).map(|b| BodyView {
            position: b.translation().to_array(),
            rotation: b.rotation().to_array(),
        })
    }

    /// Feeds the canonical state to `sink` in a fixed order.
    pub fn visit(&self, sink: &mut impl Sink) {
        sink.put("frame", 0, self.frame as u64);
        sink.put("rng", 0, self.rng.state());
        sink.put("next_crack", 0, self.next_crack as u64);

        for (i, p) in self.players.iter().enumerate() {
            let i = i as u32;
            sink.put("player.facing", i, p.facing as u64);
            sink.put("player.prev_buttons", i, p.prev_buttons as u64);
            sink.put("player.grounded", i, p.grounded as u64);
            sink.put("player.strain", i, p.strain as u64);
            sink.put("player.stun", i, p.stun as u64);
            sink.put("player.shove_cooldown", i, p.shove_cooldown as u64);
            sink.put("player.dash_cooldown", i, p.dash_cooldown as u64);
            sink.put("player.respawn_in", i, p.respawn_in as u64);
            sink.put("player.shield", i, p.shield as u64);
            sink.put("player.last_hit_by", i, p.last_hit_by as u64);
            sink.put("player.last_hit_frame", i, p.last_hit_frame as u64);
            sink.put("player.score", i, p.score as u64);
            sink.put("player.falls", i, p.falls as u64);
            self.visit_body(sink, "player.body", i, p.body);
        }

        for (i, tile) in self.tiles.iter().enumerate() {
            let i = i as u32;
            let state = match tile.state {
                TileState::Solid => 0,
                TileState::Cracking { falls_at } => 1 | (falls_at as u64) << 8,
                TileState::Falling => 2,
                TileState::Gone => 3,
            };
            sink.put("tile.state", i, state);
            if tile.state == TileState::Falling {
                self.visit_body(sink, "tile.body", i, tile.body);
            }
        }

        sink.put("props", 0, self.props.len() as u64);
        for (i, &prop) in self.props.iter().enumerate() {
            self.visit_body(sink, "prop.body", i as u32, prop);
        }
    }

    fn visit_body(
        &self,
        sink: &mut impl Sink,
        name: &'static str,
        index: u32,
        handle: RigidBodyHandle,
    ) {
        let body = &self.bodies[handle];
        let floats = body
            .translation()
            .to_array()
            .into_iter()
            .chain(body.rotation().to_array())
            .chain(body.linvel().to_array())
            .chain(body.angvel().to_array());
        // Thirteen floats, two per value: translation, rotation, linear and
        // angular velocity, as raw bits so -0.0 and NaN payloads count.
        let mut pending: Option<u32> = None;
        for f in floats {
            match pending.take() {
                None => pending = Some(f.to_bits()),
                Some(low) => sink.put(name, index, low as u64 | (f.to_bits() as u64) << 32),
            }
        }
        if let Some(low) = pending {
            sink.put(name, index, low as u64);
        }
    }

    pub fn checksum(&self) -> u64 {
        let mut hash = Fnv64::default();
        self.visit(&mut hash);
        hash.0
    }

    pub fn fields(&self) -> Vec<Field> {
        let mut fields = Vec::new();
        self.visit(&mut fields);
        fields
    }
}
