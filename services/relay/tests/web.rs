//! Player web servido pelo relay (S-011): rotas, prioridade da API e cabeçalhos.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use vellcast_relay::Relay;
use vellcast_relay::config::Config;
use vellcast_relay::web::PLAYER_CSP;

const SECRET: &[u8] = b"web-test-secret-web-test-secret-xx";

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn get(addr: SocketAddr, method: &str, path: &str) -> Resp {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines.next().unwrap()[9..12].parse().unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    Resp {
        status,
        headers,
        body: body.to_owned(),
    }
}

fn player_dir(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("vellcast-web-{name}-{}", std::process::id()));
    let dir = root.join("player");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(dir.join("assets")).unwrap();
    std::fs::write(
        dir.join("index.html"),
        "<!doctype html><title>player</title>",
    )
    .unwrap();
    std::fs::write(dir.join("assets/app.js"), "console.log(1)").unwrap();
    // Arquivo fora do diretório servido: nunca pode ser lido pelo HTTP.
    std::fs::write(root.join("secret.txt"), "segredo").unwrap();
    dir
}

async fn start(static_dir: Option<PathBuf>) -> SocketAddr {
    let mut config = Config::with_secret(SECRET.to_vec());
    config.static_dir = static_dir;
    let relay = Relay::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(vellcast_relay::serve(Arc::clone(&relay), listener));
    addr
}

#[tokio::test]
async fn serves_index_at_root_and_watch_with_security_headers() {
    let addr = start(Some(player_dir("index"))).await;
    for path in ["/", "/watch", "/watch/qualquer"] {
        let r = get(addr, "GET", path).await;
        assert_eq!(r.status, 200, "{path}");
        assert!(r.body.contains("<title>player</title>"), "{path}");
        assert_eq!(r.header("content-security-policy"), Some(PLAYER_CSP));
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
        assert_eq!(r.header("cache-control"), Some("no-cache"));
    }
}

#[tokio::test]
async fn serves_assets_without_html_policy() {
    let addr = start(Some(player_dir("assets"))).await;
    let r = get(addr, "GET", "/assets/app.js").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, "console.log(1)");
    assert!(r.header("content-type").unwrap().contains("javascript"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("content-security-policy"), None);
}

#[tokio::test]
async fn api_and_health_keep_priority() {
    let addr = start(Some(player_dir("priority"))).await;
    let r = get(addr, "GET", "/healthz").await;
    assert_eq!((r.status, r.body.as_str()), (200, "ok"));
    // Rota existente com método errado não cai no player.
    assert_eq!(get(addr, "GET", "/api/rooms").await.status, 405);
    let r = get(addr, "GET", "/metrics").await;
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().contains("json"));
}

#[tokio::test]
async fn never_serves_files_outside_the_player_dir() {
    let addr = start(Some(player_dir("traversal"))).await;
    for path in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/assets/..%2f..%2fsecret.txt",
    ] {
        let r = get(addr, "GET", path).await;
        assert!(!r.body.contains("segredo"), "{path} vazou arquivo externo");
    }
}

#[tokio::test]
async fn without_static_dir_unknown_paths_are_404() {
    let addr = start(None).await;
    assert_eq!(get(addr, "GET", "/watch").await.status, 404);
    assert_eq!(get(addr, "GET", "/healthz").await.status, 200);
}
