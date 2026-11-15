use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use fb_host::RoomOptions;
use fb_server::{Server, ServerConfig};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn config_from_env() -> Result<ServerConfig, String> {
    let port = |name: &str, default: u16| match env(name) {
        Some(value) => value
            .parse()
            .map_err(|_| format!("{name}: not a port: {value}")),
        None => Ok(default),
    };
    let public_ips = env("FB_PUBLIC_IP")
        .unwrap_or_else(|| "127.0.0.1".into())
        .split(',')
        .map(|ip| {
            ip.trim()
                .parse::<IpAddr>()
                .map_err(|_| format!("FB_PUBLIC_IP: not an address: {ip}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let any = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    Ok(ServerConfig {
        http: SocketAddr::new(any, port("FB_HTTP_PORT", 8080)?),
        udp: Some(SocketAddr::new(any, port("FB_UDP_PORT", 7600)?)),
        public_ips,
        static_dir: env("FB_STATIC").map(PathBuf::from),
        data_dir: env("FB_DATA").map(PathBuf::from),
        room: RoomOptions::default(),
    })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fb_server=info".into()),
        )
        .init();

    let config = match config_from_env() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let server = match Server::bind(config).await {
        Ok(server) => server,
        Err(error) => {
            eprintln!("could not start: {error}");
            std::process::exit(1);
        }
    };
    tracing::info!(http = %server.http, webrtc = server.webrtc, "listening");
    if let Err(error) = server.run().await {
        eprintln!("server stopped: {error}");
        std::process::exit(1);
    }
}
