//! How the server hands out the built page.

use std::time::Duration;

use fb_host::RoomOptions;
use fb_server::{Server, ServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The response head for `GET path`, lower-cased, asking for gzip.
async fn head(address: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(address).await.expect("connects");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept-Encoding: gzip\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("writes");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("reads");
    let end = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header end");
    String::from_utf8_lossy(&response[..end]).to_ascii_lowercase()
}

#[tokio::test]
async fn assets_are_compressed_and_cached_but_the_page_is_not_pinned() {
    let dir = std::env::temp_dir().join(format!("framebrawl-static-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("assets")).expect("creates the folder");
    std::fs::write(dir.join("index.html"), "<!doctype html><title>t</title>").expect("writes");
    std::fs::write(
        dir.join("assets/app-abc123.js"),
        "let x = 0;\n".repeat(4000),
    )
    .expect("writes");

    let server = Server::bind(ServerConfig {
        http: "127.0.0.1:0".parse().expect("address"),
        udp: None,
        public_ips: Vec::new(),
        static_dir: Some(dir.clone()),
        data_dir: None,
        room: RoomOptions::default(),
        silence_limit: Duration::from_secs(5),
    })
    .await
    .expect("server binds");
    let address = server.http;
    tokio::spawn(server.run());

    let asset = head(address, "/assets/app-abc123.js").await;
    assert!(asset.starts_with("http/1.1 200"), "{asset}");
    assert!(asset.contains("content-encoding: gzip"), "{asset}");
    assert!(asset.contains("immutable"), "{asset}");

    // The page itself names the current assets, so it must be fetched
    // fresh. Unknown paths fall back to it.
    for path in ["/", "/no/such/page"] {
        let page = head(address, path).await;
        assert!(page.starts_with("http/1.1 200"), "{page}");
        assert!(page.contains("content-type: text/html"), "{page}");
        assert!(!page.contains("immutable"), "{page}");
    }

    // The API is not swallowed by the file fallback.
    let health = head(address, "/api/health").await;
    assert!(
        health.contains("content-type: application/json"),
        "{health}"
    );

    std::fs::remove_dir_all(dir).ok();
}
