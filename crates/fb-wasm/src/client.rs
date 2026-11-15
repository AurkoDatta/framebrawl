//! The network client for the browser: `fb_client::Client` behind a flat,
//! number-only interface that is cheap to call from a worker.

use fb_client::{Client, Outgoing};
use fb_rollback::SessionConfig;
use fb_sim::Input;
use wasm_bindgen::prelude::*;

use crate::view::{encode_event, view_layout, write_view};

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
        view_layout()
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
    /// 1 welcome `[player, players, grid, duration, score limit]`,
    /// 2 synced `[frame]`,
    /// 3 rollback `[from, depth]`, 4 desync `[frame]`,
    /// 5 desync located `[frame]`, 6 match over `[scores...]`.
    pub fn next_event(&mut self) -> Vec<f64> {
        self.client
            .next_event()
            .map(encode_event)
            .unwrap_or_default()
    }

    /// Everything the page draws, as one flat list. Empty until the
    /// match history has been replayed.
    pub fn view(&mut self) -> Vec<f32> {
        write_view(&self.client, &mut self.view);
        self.view.clone()
    }
}
