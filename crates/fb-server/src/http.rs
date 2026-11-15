//! HTTP surface: static files, signalling, the WebSocket transport and the
//! replay API.

use std::path::PathBuf;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use str0m::change::SdpOffer;
use tower_http::services::{ServeDir, ServeFile};

use crate::replays::ReplayStore;
use crate::rooms::{RoomCommand, Rooms};
use crate::rtc::RtcEndpoint;

#[derive(Clone)]
pub struct App {
    pub rooms: Rooms,
    pub replays: ReplayStore,
    /// `None` when the UDP socket could not be bound; clients then use the
    /// WebSocket transport.
    pub rtc: Option<RtcEndpoint>,
}

#[derive(Deserialize)]
struct RoomQuery {
    room: Option<String>,
}

impl RoomQuery {
    /// Room codes are short and plain so they are safe to log and share.
    fn code(&self) -> Result<Option<&str>, StatusCode> {
        match self.room.as_deref() {
            None | Some("") => Ok(None),
            Some(code)
                if code.len() <= 16
                    && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') =>
            {
                Ok(Some(code))
            }
            Some(_) => Err(StatusCode::BAD_REQUEST),
        }
    }
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    rooms: usize,
    webrtc: bool,
    protocol: u16,
}

pub fn router(app: App, static_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/rtc", post(rtc_offer))
        .route("/ws", get(websocket))
        .route("/replays", get(list_replays))
        .route("/replays/{name}", get(read_replay))
        .with_state(app);
    let router = Router::new().nest("/api", api);
    match static_dir {
        Some(dir) => {
            let index = ServeFile::new(dir.join("index.html"));
            router.fallback_service(ServeDir::new(dir).fallback(index))
        }
        None => router,
    }
}

async fn health(State(app): State<App>) -> Json<Health> {
    Json(Health {
        status: "ok",
        rooms: app.rooms.count(),
        webrtc: app.rtc.is_some(),
        protocol: fb_net::PROTOCOL_VERSION,
    })
}

/// WebRTC signalling: the offer comes in the request, the answer goes out
/// in the response. The server is ICE lite with fixed candidates, so
/// there is nothing to trickle.
async fn rtc_offer(
    State(app): State<App>,
    Query(query): Query<RoomQuery>,
    Json(offer): Json<SdpOffer>,
) -> Response {
    let Ok(room) = query.code() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(rtc) = &app.rtc else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match rtc.accept(offer, room.map(str::to_owned)) {
        Ok(answer) => Json(answer).into_response(),
        Err(error) => {
            tracing::debug!(%error, "rejected sdp offer");
            StatusCode::BAD_REQUEST.into_response()
        }
    }
}

async fn websocket(
    State(app): State<App>,
    Query(query): Query<RoomQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let Ok(room) = query.code().map(|code| code.map(str::to_owned)) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    upgrade.on_upgrade(move |socket| serve_websocket(socket, app.rooms, room))
}

/// The fallback transport: one ordered, reliable stream carries both the
/// per-tick and the reliable messages.
async fn serve_websocket(socket: WebSocket, rooms: Rooms, room: Option<String>) {
    let Some(mut seat) = rooms.join(room.as_deref()).await else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    let player = seat.player;
    let commands = seat.commands.clone();

    let mut writer = tokio::spawn(async move {
        while let Some((_, bytes)) = seat.incoming.recv().await {
            if sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    loop {
        tokio::select! {
            // The room dropped this peer or the socket stopped accepting.
            _ = &mut writer => break,
            message = stream.next() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    let sent = commands.send(RoomCommand::Message {
                        player,
                        bytes: bytes.into(),
                    });
                    if sent.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
        }
    }
    let _ = commands.send(RoomCommand::Leave { player });
    writer.abort();
}

async fn list_replays(State(app): State<App>) -> Response {
    let replays = app.replays.clone();
    match tokio::task::spawn_blocking(move || replays.list()).await {
        Ok(entries) => Json(entries).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn read_replay(State(app): State<App>, Path(name): Path<String>) -> Response {
    let replays = app.replays.clone();
    match tokio::task::spawn_blocking(move || replays.read(&name)).await {
        Ok(Some(bytes)) => {
            ([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
