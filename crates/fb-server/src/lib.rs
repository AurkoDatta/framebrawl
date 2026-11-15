//! The framebrawl server: transports and HTTP around match rooms.

mod http;
mod replays;
mod rooms;
mod rtc;

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

use fb_host::RoomOptions;
use tokio::net::TcpListener;

pub use rtc::{CONTROL_LABEL, FRAMES_LABEL};

pub struct ServerConfig {
    pub http: SocketAddr,
    /// UDP address for WebRTC. `None` disables it.
    pub udp: Option<SocketAddr>,
    /// Addresses browsers are told to reach the UDP port on. Loopback
    /// suits Chromium and WebKit on the same machine; Firefox will not
    /// use a loopback candidate and needs a LAN address listed too.
    pub public_ips: Vec<IpAddr>,
    pub static_dir: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub room: RoomOptions,
    /// How long a seated player may send nothing before a bot takes the
    /// seat back.
    pub silence_limit: Duration,
}

pub struct Server {
    pub http: SocketAddr,
    pub webrtc: bool,
    listener: TcpListener,
    router: axum::Router,
}

impl Server {
    /// Binds the sockets. A UDP failure is not fatal: the WebSocket
    /// transport still works.
    pub async fn bind(config: ServerConfig) -> std::io::Result<Server> {
        let replays = replays::ReplayStore::new(config.data_dir.map(|dir| dir.join("replays")));
        let rooms = rooms::Rooms::new(config.room, config.silence_limit, replays.clone());

        let rtc = config.udp.and_then(|bind| {
            let candidates = config
                .public_ips
                .iter()
                .map(|&ip| SocketAddr::new(ip, bind.port()))
                .collect();
            match rtc::RtcEndpoint::start(bind, candidates, rooms.clone()) {
                Ok(endpoint) => Some(endpoint),
                Err(error) => {
                    tracing::warn!(%bind, %error, "WebRTC disabled: could not bind UDP");
                    None
                }
            }
        });

        let listener = TcpListener::bind(config.http).await?;
        let webrtc = rtc.is_some();
        let app = http::App {
            rooms,
            replays,
            rtc,
        };
        Ok(Server {
            http: listener.local_addr()?,
            webrtc,
            listener,
            router: http::router(app, config.static_dir),
        })
    }

    pub async fn run(self) -> std::io::Result<()> {
        axum::serve(self.listener, self.router)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await
    }
}
