//! Testes de integração da lista de participantes (nome do convidado + `Presence`):
//! relay real em porta efêmera, clientes WebSocket e HTTP reais.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use vellcast_protocol::messages::{ParticipantRole, Presence};
use vellcast_protocol::{HEADER_LEN, Header, MAX_MESSAGE_LEN, MessageType, decode, encode};
use vellcast_relay::Relay;
use vellcast_relay::config::Config;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SECRET: &[u8] = b"presence-secret-presence-secret-x";
const WAIT: Duration = Duration::from_secs(5);

async fn start() -> (SocketAddr, Arc<Relay>) {
    let relay = Relay::new(Config::with_secret(SECRET.to_vec()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(vellcast_relay::serve(Arc::clone(&relay), listener));
    (addr, relay)
}

/// Cliente HTTP/1.1 mínimo (evita dependências só para testes).
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
        Some(json!({"name": "Presença"})),
    )
    .await;
    assert_eq!(status, 201, "{v}");
    Room {
        id: v["room"]["id"].as_str().unwrap().into(),
        owner: v["ownerToken"].as_str().unwrap().into(),
        publisher: v["publisherToken"].as_str().unwrap().into(),
    }
}

async fn invite(addr: SocketAddr, room: &Room) -> String {
    let (s, inv) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/invites", room.id),
        Some(&room.owner),
        Some(json!({ "maxUses": 1 })),
    )
    .await;
    assert_eq!(s, 201, "{inv}");
    inv["code"].as_str().unwrap().into()
}

async fn join(addr: SocketAddr, room: &Room, body: Value) -> (u16, Value) {
    http(
        addr,
        "POST",
        &format!("/api/rooms/{}/join", room.id),
        None,
        Some(body),
    )
    .await
}

async fn named_viewer_token(addr: SocketAddr, room: &Room, name: &str) -> String {
    let code = invite(addr, room).await;
    let (s, joined) = join(addr, room, json!({ "invite": code, "name": name })).await;
    assert_eq!(s, 200, "{joined}");
    joined["viewerToken"].as_str().unwrap().into()
}

async fn connect(addr: SocketAddr, auth: Value) -> Ws {
    let mut ws = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap()
        .0;
    let msg = encode(
        &Header::new(MessageType::Auth, 0),
        auth.to_string().as_bytes(),
    )
    .unwrap();
    ws.send(Message::Binary(msg)).await.unwrap();
    let (h, _) = recv(&mut ws).await.expect("resposta de auth");
    assert_eq!(h.kind, MessageType::AuthOk);
    ws
}

async fn recv(ws: &mut Ws) -> Option<(Header, Bytes)> {
    loop {
        match tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("timeout esperando mensagem")
        {
            Some(Ok(Message::Binary(d))) => {
                let (h, _) = decode(&d, MAX_MESSAGE_LEN).unwrap();
                return Some((h, d));
            }
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

/// Próxima lista de participantes recebida, ignorando outras mensagens.
async fn next_presence(ws: &mut Ws) -> (Presence, String) {
    loop {
        let (h, d) = recv(ws).await.expect("conexão aberta");
        if h.kind == MessageType::Presence {
            let raw = String::from_utf8(d[HEADER_LEN..].to_vec()).unwrap();
            return (serde_json::from_str(&raw).unwrap(), raw);
        }
    }
}

/// Lê listas até chegar uma com `count` participantes.
async fn presence_with(ws: &mut Ws, count: usize) -> (Presence, String) {
    loop {
        let p = next_presence(ws).await;
        if p.0.count == count {
            return p;
        }
    }
}

fn names(p: &Presence) -> Vec<(ParticipantRole, Option<&str>)> {
    p.participants
        .iter()
        .map(|x| (x.role, x.name.as_deref()))
        .collect()
}

#[tokio::test]
async fn invite_is_consumed_only_with_a_valid_name() {
    let (addr, _) = start().await;
    let room = create_room(addr).await;
    let code = invite(addr, &room).await;

    for bad in [
        json!(""),
        json!("   \t"),
        json!("\u{0007}"),
        json!("x".repeat(33)),
    ] {
        let (s, v) = join(addr, &room, json!({ "invite": code, "name": bad })).await;
        assert_eq!(s, 400, "nome {bad} deveria ser recusado: {v}");
    }
    // O convite de uso único continua valendo depois das recusas.
    let (s, v) = join(addr, &room, json!({ "invite": code, "name": "  Ana  " })).await;
    assert_eq!(s, 200, "{v}");
    assert!(v["viewerToken"].is_string());
    let (s, _) = join(addr, &room, json!({ "invite": code, "name": "Ana" })).await;
    assert_eq!(s, 403, "agora sim o convite foi consumido");

    // Clientes antigos (sem `name`) continuam entrando.
    let code = invite(addr, &room).await;
    let (s, _) = join(addr, &room, json!({ "invite": code })).await;
    assert_eq!(s, 200);
}

#[tokio::test]
async fn presence_tracks_join_leave_and_reconnect() {
    let (addr, relay) = start().await;
    let room = create_room(addr).await;

    let _publisher = connect(
        addr,
        json!({"token": room.publisher, "role": "publisher", "name": "Host\u{0007}"}),
    )
    .await;
    let ana_token = named_viewer_token(addr, &room, "Ana").await;
    let bia_token = named_viewer_token(addr, &room, "<b>Bia</b>").await;

    let mut ana = connect(addr, json!({"token": ana_token, "role": "viewer"})).await;
    let (p, _) = presence_with(&mut ana, 2).await;
    assert_eq!(
        names(&p),
        vec![
            (ParticipantRole::Publisher, Some("Host")),
            (ParticipantRole::Viewer, Some("Ana")),
        ]
    );

    // O nome do espectador vem do token: o `Auth` não consegue trocá-lo.
    let mut bia = connect(
        addr,
        json!({"token": bia_token, "role": "viewer", "name": "Impostor"}),
    )
    .await;
    let (p, raw) = presence_with(&mut ana, 3).await;
    assert_eq!(p.participants[2].name.as_deref(), Some("<b>Bia</b>"));
    for secret in [&ana_token, &bia_token, &room.publisher, &room.owner] {
        assert!(!raw.contains(secret.as_str()), "a lista não expõe tokens");
    }
    let bia_id = p.participants[2].id.clone();
    let (p, _) = presence_with(&mut bia, 3).await;
    assert_eq!(p.participants[2].id, bia_id);

    // Bia cai: a lista volta a 2.
    drop(bia);
    let (p, _) = presence_with(&mut ana, 2).await;
    assert!(p.participants.iter().all(|x| x.id != bia_id));

    // Reconexão com o mesmo token: uma entrada só, mesmo com duas conexões abertas.
    let bia1 = connect(addr, json!({"token": bia_token, "role": "viewer"})).await;
    let (p, _) = presence_with(&mut ana, 3).await;
    let bia2 = connect(addr, json!({"token": bia_token, "role": "viewer"})).await;
    let live = relay.room(&room.id).unwrap();
    assert_eq!(live.presence().count, 3);
    assert_eq!(
        p.participants
            .iter()
            .filter(|x| x.name.as_deref() == Some("<b>Bia</b>"))
            .count(),
        1
    );
    drop(bia1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        live.presence().count,
        3,
        "a segunda conexão ainda está ativa"
    );
    drop(bia2);
    presence_with(&mut ana, 2).await;

    // O transmissor sai: a lista mostra só a Ana.
    drop(_publisher);
    let (p, _) = presence_with(&mut ana, 1).await;
    assert_eq!(names(&p), vec![(ParticipantRole::Viewer, Some("Ana"))]);
}

#[tokio::test]
async fn viewers_without_name_are_listed_anonymously() {
    let (addr, _) = start().await;
    let room = create_room(addr).await;
    let code = invite(addr, &room).await;
    let (_, joined) = join(addr, &room, json!({ "invite": code })).await;
    let token = joined["viewerToken"].as_str().unwrap();
    let mut ws = connect(addr, json!({"token": token, "role": "viewer"})).await;
    let (p, _) = presence_with(&mut ws, 1).await;
    assert_eq!(names(&p), vec![(ParticipantRole::Viewer, None)]);
}
