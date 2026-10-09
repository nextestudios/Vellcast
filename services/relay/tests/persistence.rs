//! Integração: salas persistidas em SQLite, revogação de tokens e rotação do segredo.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use vellcast_protocol::messages::{AuthRequest, ConnectRole, ErrorBody, ErrorCode};
use vellcast_protocol::{Header, MAX_MESSAGE_LEN, MessageType, decode, encode};
use vellcast_relay::Relay;
use vellcast_relay::config::Config;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SECRET: &[u8] = b"persistence-secret-persistence-sec";
const OLD_SECRET: &[u8] = b"old-secret-old-secret-old-secret-xy";
const WAIT: Duration = Duration::from_secs(5);

async fn start(
    config: Config,
) -> (
    SocketAddr,
    Arc<Relay>,
    tokio::task::JoinHandle<std::io::Result<()>>,
) {
    let relay = Relay::from_config(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(vellcast_relay::serve(Arc::clone(&relay), listener));
    (addr, relay, task)
}

fn config(secret: &[u8], db: Option<PathBuf>) -> Config {
    let mut c = Config::with_secret(secret.to_vec());
    c.db_path = db;
    c.cleanup_interval = Duration::from_millis(50);
    c
}

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vellcast-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("relay.db")
}

async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    if let Some(t) = bearer {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    let status = text[9..12].parse().unwrap();
    let json = text
        .split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    (status, json)
}

struct Room {
    id: String,
    owner: String,
    publisher: String,
}

async fn create_room(addr: SocketAddr) -> Room {
    let (status, v) = http(
        addr,
        "POST",
        "/api/rooms",
        None,
        Some(json!({ "name": "sala" })),
    )
    .await;
    assert_eq!(status, 201, "{v}");
    Room {
        id: v["room"]["id"].as_str().unwrap().into(),
        owner: v["ownerToken"].as_str().unwrap().into(),
        publisher: v["publisherToken"].as_str().unwrap().into(),
    }
}

async fn invite(addr: SocketAddr, room: &Room, uses: u32) -> String {
    let (s, inv) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/invites", room.id),
        Some(&room.owner),
        Some(json!({ "maxUses": uses })),
    )
    .await;
    assert_eq!(s, 201, "{inv}");
    inv["code"].as_str().unwrap().into()
}

async fn join(addr: SocketAddr, room: &Room, code: &str) -> (u16, Value) {
    http(
        addr,
        "POST",
        &format!("/api/rooms/{}/join", room.id),
        None,
        Some(json!({ "invite": code })),
    )
    .await
}

async fn auth(addr: SocketAddr, token: &str, role: ConnectRole) -> Ws {
    let mut ws = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap()
        .0;
    let req = AuthRequest {
        token: token.into(),
        role,
    };
    let msg = encode(
        &Header::new(MessageType::Auth, 0),
        &serde_json::to_vec(&req).unwrap(),
    )
    .unwrap();
    ws.send(Message::Binary(msg)).await.unwrap();
    ws
}

/// Próxima mensagem binária do relay (ignora Pong); `None` quando a conexão fecha.
async fn recv(ws: &mut Ws) -> Option<(Header, Bytes)> {
    loop {
        match tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("timeout esperando mensagem")
        {
            Some(Ok(Message::Binary(d))) => {
                let (h, _) = decode(&d, MAX_MESSAGE_LEN).unwrap();
                if h.kind == MessageType::Pong {
                    continue;
                }
                return Some((h, d));
            }
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

/// Espera um `Error` com o código dado, pulando outras mensagens.
async fn expect_error(ws: &mut Ws, code: ErrorCode) {
    loop {
        let (h, d) = recv(ws).await.expect("conexão fechou sem Error");
        if h.kind == MessageType::Error {
            let body: ErrorBody =
                serde_json::from_slice(&d[vellcast_protocol::HEADER_LEN..]).unwrap();
            assert_eq!(body.code, code, "{}", body.message);
            return;
        }
    }
}

async fn expect_auth_ok(ws: &mut Ws) {
    let (h, _) = recv(ws).await.expect("AuthOk");
    assert_eq!(h.kind, MessageType::AuthOk);
}

#[tokio::test]
async fn rooms_and_invites_survive_relay_restart() {
    let db = temp_db("restart");
    let (addr, _relay, task) = start(config(SECRET, Some(db.clone()))).await;
    let room = create_room(addr).await;
    let code = invite(addr, &room, 2).await;
    assert_eq!(join(addr, &room, &code).await.0, 200, "primeiro uso");
    task.abort();
    let _ = task.await;

    // Mesmo banco e mesmo segredo: sala, tokens e o uso restante do convite continuam valendo.
    let (addr, _relay, _task) = start(config(SECRET, Some(db.clone()))).await;
    let (s, v) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["name"], "sala");
    assert_eq!(
        join(addr, &room, &code).await.0,
        200,
        "segundo uso persistido"
    );
    assert_eq!(join(addr, &room, &code).await.0, 403, "convite esgotado");
    let mut publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    expect_auth_ok(&mut publisher).await;
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

#[tokio::test]
async fn memory_relay_forgets_rooms_on_restart() {
    let (addr, _relay, task) = start(config(SECRET, None)).await;
    let room = create_room(addr).await;
    task.abort();
    let _ = task.await;
    let (addr, _relay, _task) = start(config(SECRET, None)).await;
    let (s, _) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 404);
}

#[tokio::test]
async fn revoking_all_viewers_disconnects_them_and_invalidates_tokens() {
    let (addr, _relay, _task) = start(config(SECRET, None)).await;
    let room = create_room(addr).await;
    let code = invite(addr, &room, 3).await;
    let (_, joined) = join(addr, &room, &code).await;
    let viewer_token = joined["viewerToken"].as_str().unwrap().to_owned();
    let mut viewer = auth(addr, &viewer_token, ConnectRole::Viewer).await;
    expect_auth_ok(&mut viewer).await;

    let (s, v) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.owner),
        Some(json!({ "allViewers": true })),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    expect_error(&mut viewer, ErrorCode::Unauthorized).await;

    let mut again = auth(addr, &viewer_token, ConnectRole::Viewer).await;
    expect_error(&mut again, ErrorCode::Unauthorized).await;
    let (s, _) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&viewer_token),
        None,
    )
    .await;
    assert_eq!(s, 401, "token revogado também na API");

    // Tokens emitidos depois da revogação valem (a revogação cobre o segundo corrente).
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let (s, fresh) = join(addr, &room, &code).await;
    assert_eq!(s, 200);
    let mut fresh_ws = auth(
        addr,
        fresh["viewerToken"].as_str().unwrap(),
        ConnectRole::Viewer,
    )
    .await;
    expect_auth_ok(&mut fresh_ws).await;
}

#[tokio::test]
async fn revoking_a_publisher_token_kicks_it_and_blocks_reconnect() {
    let (addr, _relay, _task) = start(config(SECRET, None)).await;
    let room = create_room(addr).await;
    let mut publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    expect_auth_ok(&mut publisher).await;

    // Token de outra sala e pedido vazio são recusados.
    let other = create_room(addr).await;
    let (s, _) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.owner),
        Some(json!({ "token": other.publisher })),
    )
    .await;
    assert_eq!(s, 400);
    let (s, _) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.owner),
        Some(json!({})),
    )
    .await;
    assert_eq!(s, 400);
    // Só o dono revoga.
    let (s, _) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.publisher),
        Some(json!({ "token": room.publisher })),
    )
    .await;
    assert_eq!(s, 403);

    let (s, v) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.owner),
        Some(json!({ "token": room.publisher })),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    expect_error(&mut publisher, ErrorCode::Unauthorized).await;
    let mut again = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    expect_error(&mut again, ErrorCode::Unauthorized).await;

    // Um token novo de transmissor (emitido pelo dono) funciona.
    let (s, v) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/publisher-token", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 200);
    let mut fresh = auth(
        addr,
        v["publisherToken"].as_str().unwrap(),
        ConnectRole::Publisher,
    )
    .await;
    expect_auth_ok(&mut fresh).await;
}

#[tokio::test]
async fn revocations_persist_across_restart() {
    let db = temp_db("revoke");
    let (addr, _relay, task) = start(config(SECRET, Some(db.clone()))).await;
    let room = create_room(addr).await;
    let (s, _) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.owner),
        Some(json!({ "token": room.publisher })),
    )
    .await;
    assert_eq!(s, 200);
    task.abort();
    let _ = task.await;
    let (addr, _relay, _task) = start(config(SECRET, Some(db.clone()))).await;
    let mut publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    expect_error(&mut publisher, ErrorCode::Unauthorized).await;
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}

#[tokio::test]
async fn secret_rotation_accepts_previous_secret_without_downtime() {
    let db = temp_db("rotation");
    // Relay com o segredo antigo emite os tokens da sala.
    let (addr, _relay, task) = start(config(OLD_SECRET, Some(db.clone()))).await;
    let room = create_room(addr).await;
    task.abort();
    let _ = task.await;

    // Rotação: segredo novo + anterior aceito só para verificar.
    let mut rotated = config(SECRET, Some(db.clone()));
    rotated.previous_secret = Some(OLD_SECRET.to_vec());
    let (addr, _relay, task) = start(rotated).await;
    let (s, _) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 200, "token antigo aceito durante a rotação");
    let (s, v) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/publisher-token", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 200);
    let new_publisher = v["publisherToken"].as_str().unwrap().to_owned();
    task.abort();
    let _ = task.await;

    // Fim da rotação: sem o anterior, tokens antigos caem e os novos continuam.
    let (addr, _relay, _task) = start(config(SECRET, Some(db.clone()))).await;
    let (s, _) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 401, "token antigo recusado após a rotação");
    let mut publisher = auth(addr, &new_publisher, ConnectRole::Publisher).await;
    expect_auth_ok(&mut publisher).await;
    let _ = std::fs::remove_dir_all(db.parent().unwrap());
}
