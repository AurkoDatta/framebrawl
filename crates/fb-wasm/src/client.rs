//! The network client for the browser: `fb_client::Client` behind a flat,
//! number-only interface that is cheap to call from a worker.

use fb_client::{Client, Event, Outgoing};
use fb_rollback::SessionConfig;
use fb_sim::{BodyView, Input, TileState, World};
use wasm_bindgen::prelude::*;

/// Floats per section of the view written by [`NetClient::view`]. The
/// TypeScript side reads the same numbers from `view_layout`.
const HEADER: usize = 16;
const PER_PLAYER: usize = 12;
const PER_TILE: usize = 8;
const PER_PROP: usize = 8;

#[wasm_bindgen]
pub struct NetClient {
    client: Client,
    outgoing: Vec<Outgoing>,
    view: Vec<f32>,
}

impl Default for NetClient {
    fn default() -> Self {
        NetClient::new()
    }
}

#[wasm_bindgen]
impl NetClient {
    #[wasm_bindgen(constructor)]
    pub fn new() -> NetClient {
        NetClient {
            client: Client::new(SessionConfig::default()),
            outgoing: Vec::new(),
            view: Vec::new(),
        }
    }

    /// `[header, per player, per tile, per prop]` float counts.
    pub fn view_layout() -> Vec<u32> {
        [HEADER, PER_PLAYER, PER_TILE, PER_PROP]
            .map(|n| n as u32)
            .to_vec()
    }

    /// Handles one message from the server.
    pub fn receive(&mut self, bytes: &[u8], now_ms: u32) -> Result<(), JsError> {
        self.client
            .receive(bytes, now_ms)
            .map_err(|error| JsError::new(&error.to_string()))
    }

    /// One 60 Hz tick. Returns how many messages are waiting to be sent;
    /// read them with `outgoing_reliable` and `outgoing_bytes`.
    pub fn tick(&mut self, buttons: u8, aim: u8, now_ms: u32) -> u32 {
        self.outgoing = self.client.tick(Input::new(buttons, aim), now_ms);
        self.outgoing.len() as u32
    }

    pub fn outgoing_reliable(&self, index: u32) -> bool {
        self.outgoing[index as usize].reliable
    }

    pub fn outgoing_bytes(&self, index: u32) -> Vec<u8> {
        self.outgoing[index as usize].bytes.clone()
    }

    /// The next event as `[kind, ...numbers]`, or an empty list.
    ///
    /// 1 welcome `[player, players, grid]`, 2 synced `[frame]`,
    /// 3 rollback `[from, depth]`, 4 desync `[frame]`,
    /// 5 desync located `[frame]`, 6 match over `[scores...]`.
    pub fn next_event(&mut self) -> Vec<f64> {
        let Some(event) = self.client.next_event() else {
            return Vec::new();
        };
        match event {
            Event::Welcome { player, config } => vec![
                1.0,
                f64::from(player),
                f64::from(config.players),
                f64::from(config.grid),
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

    /// Everything the page draws, as one flat list. Empty until the
    /// match history has been replayed.
    pub fn view(&mut self) -> Vec<f32> {
        self.view.clear();
        let Some(world) = self.client.world() else {
            return Vec::new();
        };
        let stats = self.client.stats();
        let props = world.props().count();
        self.view.extend([
            world.frame() as f32,
            self.client.confirmed_frame() as f32,
            f32::from(self.client.player().unwrap_or(0)),
            world.player_count() as f32,
            world.tile_count() as f32,
            props as f32,
            self.client.rtt_ms() as f32,
            f32::from(self.client.input_delay()),
            stats.rollbacks as f32,
            stats.resimulated_frames as f32,
            stats.deepest_rollback as f32,
            stats.stalls as f32,
            stats.catch_up_frames as f32,
            stats.substituted_inputs as f32,
            stats.checkpoints_verified as f32,
            f32::from(u8::from(world.finished())),
        ]);
        write_world(world, &mut self.view);
        self.view.clone()
    }
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
