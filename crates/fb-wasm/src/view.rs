//! The flat number lists that cross from wasm to the page.

use fb_client::{Client, Event};
use fb_sim::{BodyView, TileState, World};

/// Floats per section of a view. The TypeScript side reads the same
/// numbers from `view_layout`.
const HEADER: usize = 16;
const PER_PLAYER: usize = 12;
const PER_TILE: usize = 8;
const PER_PROP: usize = 8;

/// `[header, per player, per tile, per prop]` float counts.
pub fn view_layout() -> Vec<u32> {
    [HEADER, PER_PLAYER, PER_TILE, PER_PROP]
        .map(|n| n as u32)
        .to_vec()
}

/// An event as `[kind, ...numbers]`.
///
/// 1 welcome `[player, players, grid, duration, score limit]`,
/// 2 synced `[frame]`, 3 rollback `[from, depth]`, 4 desync `[frame]`,
/// 5 desync located `[frame]`, 6 match over `[scores...]`.
pub fn encode_event(event: Event) -> Vec<f64> {
    match event {
        Event::Welcome { player, config } => vec![
            1.0,
            f64::from(player),
            f64::from(config.players),
            f64::from(config.grid),
            f64::from(config.duration_frames),
            f64::from(config.score_limit),
        ],
        Event::Synced { frame } => vec![2.0, f64::from(frame)],
        Event::Rollback(rollback) => {
            vec![3.0, f64::from(rollback.from), f64::from(rollback.depth)]
        }
        Event::Desync { frame } => vec![4.0, f64::from(frame)],
        Event::DesyncFound { frame } => vec![5.0, f64::from(frame)],
        Event::MatchOver { scores, .. } => std::iter::once(6.0)
            .chain(scores.into_iter().map(f64::from))
            .collect(),
    }
}

/// Everything the page draws of the client's predicted world, as one flat
/// list. Left empty until the match history has been replayed.
pub fn write_view(client: &Client, out: &mut Vec<f32>) {
    out.clear();
    let Some(world) = client.world() else {
        return;
    };
    let stats = client.stats();
    out.extend([
        world.frame() as f32,
        client.confirmed_frame() as f32,
        f32::from(client.player().unwrap_or(0)),
        world.player_count() as f32,
        world.tile_count() as f32,
        world.props().count() as f32,
        client.rtt_ms() as f32,
        f32::from(client.input_delay()),
        stats.rollbacks as f32,
        stats.resimulated_frames as f32,
        stats.deepest_rollback as f32,
        stats.stalls as f32,
        stats.catch_up_frames as f32,
        stats.substituted_inputs as f32,
        stats.checkpoints_verified as f32,
        f32::from(u8::from(world.finished())),
    ]);
    write_world(world, out);
}

/// A world with nobody predicting it, as when watching a replay. The seat
/// is one no player has, and every netcode figure is zero except the
/// count of verified checkpoints.
pub fn write_spectator_view(world: &World, checkpoints: u32, out: &mut Vec<f32>) {
    out.clear();
    let mut header = [0.0; HEADER];
    header[0] = world.frame() as f32;
    header[1] = world.frame() as f32;
    header[2] = 255.0;
    header[3] = world.player_count() as f32;
    header[4] = world.tile_count() as f32;
    header[5] = world.props().count() as f32;
    header[14] = checkpoints as f32;
    header[15] = f32::from(u8::from(world.finished()));
    out.extend(header);
    write_world(world, out);
}

fn write_world(world: &World, out: &mut Vec<f32>) {
    for index in 0..world.player_count() {
        let p = world.player(index);
        let flags = u8::from(p.in_play)
            | u8::from(p.grounded) << 1
            | u8::from(p.shielded) << 2
            | u8::from(p.stunned) << 3;
        out.extend(p.position);
        out.extend(p.velocity);
        out.extend([
            f32::from(p.facing),
            f32::from(flags),
            f32::from(p.strain),
            f32::from(p.score),
            f32::from(p.shove_cooldown),
            f32::from(p.dash_cooldown),
        ]);
    }
    for index in 0..world.tile_count() {
        let (state, body) = world.tile(index);
        out.push(match state {
            TileState::Solid => 0.0,
            TileState::Cracking { .. } => 1.0,
            TileState::Falling => 2.0,
            TileState::Gone => 3.0,
        });
        write_body(body, out);
    }
    for (prop, round) in world.props().zip(world.prop_is_round()) {
        out.push(f32::from(u8::from(round)));
        write_body(Some(prop), out);
    }
}

fn write_body(body: Option<BodyView>, out: &mut Vec<f32>) {
    match body {
        Some(body) => {
            out.extend(body.position);
            out.extend(body.rotation);
        }
        None => out.extend([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]),
    }
}
