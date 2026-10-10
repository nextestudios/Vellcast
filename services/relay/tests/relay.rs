//! Testes de integração: relay real em porta efêmera, clientes WebSocket e HTTP reais.

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
use vellcast_protocol::messages::{AuthRequest, ConnectRole, ErrorBody, ErrorCode};
use vellcast_protocol::{Header, MAX_MESSAGE_LEN, MessageType, decode, encode, flags};
use vellcast_relay::config::Config;
use vellcast_relay::token::Role;
use vellcast_relay::{Relay, unix_now};
use vellcast_transport::{Publisher, PublisherConfig};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SECRET: &[u8] = b"integration-secret-integration-se";
const WAIT: Duration = Duration::from_secs(5);

async fn start(tweak: impl FnOnce(&mut Config)) -> (SocketAddr, Arc<Relay>) {
    let mut config = Config::with_secret(SECRET.to_vec());
    config.cleanup_interval = Duration::from_millis(50);
    tweak(&mut config);
    let relay = Relay::new(config);
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

async fn create_room(addr: SocketAddr, body: Value) -> Room {
    let (status, v) = http(addr, "POST", "/api/rooms", None, Some(body)).await;
    assert_eq!(status, 201, "{v}");
    Room {
        id: v["room"]["id"].as_str().unwrap().into(),
        owner: v["ownerToken"].as_str().unwrap().into(),
        publisher: v["publisherToken"].as_str().unwrap().into(),
    }
}

async fn viewer_token(addr: SocketAddr, room: &Room) -> String {
    let (s, inv) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/invites", room.id),
        Some(&room.owner),
        Some(json!({ "maxUses": 1 })),
    )
    .await;
    assert_eq!(s, 201, "{inv}");
    let (s, joined) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/join", room.id),
        None,
        Some(json!({ "invite": inv["code"] })),
    )
    .await;
    assert_eq!(s, 200, "{joined}");
    joined["viewerToken"].as_str().unwrap().into()
}

async fn connect(addr: SocketAddr) -> Ws {
    tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap()
        .0
}

async fn send(ws: &mut Ws, data: Bytes) {
    ws.send(Message::Binary(data)).await.unwrap();
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

/// Próxima mensagem cujo tipo não seja Pong nem Presence (coberta em `tests/presence.rs`).
async fn recv_kind(ws: &mut Ws) -> Option<(Header, Bytes)> {
    loop {
        let m = recv(ws).await?;
        if !matches!(m.0.kind, MessageType::Pong | MessageType::Presence) {
            return Some(m);
        }
    }
}

fn auth_msg(token: &str, role: ConnectRole) -> Bytes {
    let body = serde_json::to_vec(&AuthRequest {
        token: token.into(),
        role,
    })
    .unwrap();
    encode(&Header::new(MessageType::Auth, 0), &body).unwrap()
}

async fn auth(addr: SocketAddr, token: &str, role: ConnectRole) -> Ws {
    let mut ws = connect(addr).await;
    send(&mut ws, auth_msg(token, role)).await;
    let (h, d) = recv(&mut ws).await.expect("resposta de auth");
    assert_eq!(h.kind, MessageType::AuthOk, "{:?}", error_of(&d));
    ws
}

fn error_of(data: &Bytes) -> Option<ErrorBody> {
    serde_json::from_slice(&data[vellcast_protocol::HEADER_LEN..]).ok()
}

async fn expect_error(ws: &mut Ws, code: ErrorCode) {
    let (h, d) = recv_kind(ws).await.expect("mensagem de erro");
    assert_eq!(h.kind, MessageType::Error);
    assert_eq!(error_of(&d).unwrap().code, code);
}

fn media(kind: MessageType, seq: u32, keyframe: bool, payload: &[u8]) -> Bytes {
    let mut h = Header::new(kind, 0);
    h.sequence = seq;
    h.pts_us = u64::from(seq) * 16_666;
    h.sent_at_us = 42;
    if keyframe {
        h.flags = flags::KEYFRAME;
    }
    encode(&h, payload).unwrap()
}

fn video_init() -> Bytes {
    let body =
        json!({"codec": "avc1.64002A", "codedWidth": 1920, "codedHeight": 1080, "framerate": 60});
    encode(
        &Header::new(MessageType::VideoInit, 0),
        body.to_string().as_bytes(),
    )
    .unwrap()
}

fn audio_init() -> Bytes {
    let body = json!({"codec": "opus", "sampleRate": 48000, "numberOfChannels": 2});
    encode(
        &Header::new(MessageType::AudioInit, 0),
        body.to_string().as_bytes(),
    )
    .unwrap()
}

#[tokio::test]
async fn fanout_delivers_byte_identical_messages_to_all_viewers() {
    let (addr, relay) = start(|_| {}).await;
    let room = create_room(addr, json!({"name": "Sala de teste"})).await;
    let mut publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    send(&mut publisher, video_init()).await;
    send(&mut publisher, audio_init()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut viewers = Vec::new();
    for _ in 0..3 {
        let token = viewer_token(addr, &room).await;
        viewers.push(auth(addr, &token, ConnectRole::Viewer).await);
    }

    let mut sent = Vec::new();
    for i in 0..30u32 {
        let v = media(
            MessageType::VideoFrame,
            i,
            i % 10 == 0,
            &vec![i as u8; 1000 + i as usize],
        );
        let a = media(MessageType::AudioFrame, i, false, &[0xAA; 80]);
        send(&mut publisher, v.clone()).await;
        send(&mut publisher, a.clone()).await;
        sent.push(v);
        sent.push(a);
    }

    for ws in &mut viewers {
        let mut got = Vec::new();
        while got.len() < sent.len() {
            let (h, d) = recv_kind(ws).await.expect("mídia");
            if matches!(h.kind, MessageType::VideoFrame | MessageType::AudioFrame) {
                got.push(d);
            }
        }
        assert_eq!(
            got, sent,
            "o relay entrega exatamente os bytes do transmissor"
        );
    }
    let m = relay.metrics_snapshot();
    assert_eq!(m.viewers, 3);
    assert_eq!(m.rooms_live, 1);
}

#[tokio::test]
async fn new_viewer_starts_at_cached_keyframe() {
    let (addr, _) = start(|_| {}).await;
    let room = create_room(addr, json!({"name": "gop"})).await;
    let mut publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    send(&mut publisher, video_init()).await;
    send(
        &mut publisher,
        media(MessageType::VideoFrame, 0, false, b"delta-orfao"),
    )
    .await;
    let key = media(MessageType::VideoFrame, 1, true, b"keyframe");
    send(&mut publisher, key.clone()).await;
    let mut deltas = Vec::new();
    for i in 2..6 {
        let d = media(MessageType::VideoFrame, i, false, b"delta");
        send(&mut publisher, d.clone()).await;
        deltas.push(d);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let token = viewer_token(addr, &room).await;
    let mut viewer = auth(addr, &token, ConnectRole::Viewer).await;
    let (h, _) = recv_kind(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::VideoInit);
    let (h, d) = recv_kind(&mut viewer).await.unwrap();
    assert!(h.is_keyframe(), "primeiro quadro é o keyframe em cache");
    assert_eq!(d, key);
    for expected in deltas {
        assert_eq!(recv_kind(&mut viewer).await.unwrap().1, expected);
    }
}

#[tokio::test]
async fn rejects_unauthorized_connections() {
    let (addr, relay) = start(|c| c.auth_timeout = Duration::from_millis(300)).await;
    let room = create_room(addr, json!({"name": "auth"})).await;

    // Sem Auth dentro do prazo.
    let mut ws = connect(addr).await;
    expect_error(&mut ws, ErrorCode::AuthTimeout).await;

    // Primeira mensagem não é Auth.
    let mut ws = connect(addr).await;
    send(
        &mut ws,
        encode(&Header::new(MessageType::Ping, 0), &[0; 8]).unwrap(),
    )
    .await;
    expect_error(&mut ws, ErrorCode::ProtocolError).await;

    // Token adulterado.
    let mut ws = connect(addr).await;
    let mut forged = room.publisher.clone();
    // Troca o último caractere por outro diferente: com base64 sem padding, o
    // último caractere da assinatura pode já ser 'A' (~1/16), o que deixaria o
    // token intacto e o teste instável.
    let last = forged.pop().unwrap_or('A');
    forged.push(if last == 'A' { 'B' } else { 'A' });
    send(&mut ws, auth_msg(&forged, ConnectRole::Publisher)).await;
    expect_error(&mut ws, ErrorCode::Unauthorized).await;

    // Token expirado.
    let (expired, _) = relay
        .signer
        .issue(&room.id, Role::Viewer, Duration::ZERO, unix_now());
    let mut ws = connect(addr).await;
    send(&mut ws, auth_msg(&expired, ConnectRole::Viewer)).await;
    expect_error(&mut ws, ErrorCode::Unauthorized).await;

    // Token de espectador tentando transmitir.
    let viewer = viewer_token(addr, &room).await;
    let mut ws = connect(addr).await;
    send(&mut ws, auth_msg(&viewer, ConnectRole::Publisher)).await;
    expect_error(&mut ws, ErrorCode::Forbidden).await;

    // Token de outra sala (já encerrada).
    let other = create_room(addr, json!({"name": "outra"})).await;
    let (s, _) = http(
        addr,
        "DELETE",
        &format!("/api/rooms/{}", other.id),
        Some(&other.owner),
        None,
    )
    .await;
    assert_eq!(s, 204);
    let mut ws = connect(addr).await;
    send(&mut ws, auth_msg(&other.publisher, ConnectRole::Publisher)).await;
    expect_error(&mut ws, ErrorCode::RoomNotFound).await;

    // HTTP: token de uma sala não vale para outra; espectador não gerencia.
    let third = create_room(addr, json!({"name": "terceira"})).await;
    let (s, _) = http(
        addr,
        "DELETE",
        &format!("/api/rooms/{}", third.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 401);
    let (s, _) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/invites", room.id),
        Some(&viewer),
        None,
    )
    .await;
    assert_eq!(s, 403);
    assert!(relay.metrics_snapshot().auth_failures >= 6);
}

#[tokio::test]
async fn rejects_oversized_messages() {
    let (addr, _) = start(|_| {}).await;
    let room = create_room(addr, json!({"name": "limites"})).await;
    let token = viewer_token(addr, &room).await;
    let mut viewer = auth(addr, &token, ConnectRole::Viewer).await;
    send(
        &mut viewer,
        encode(&Header::new(MessageType::Ping, 0), &[0; 4096]).unwrap(),
    )
    .await;
    expect_error(&mut viewer, ErrorCode::MessageTooLarge).await;

    let mut publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    let huge = vec![0u8; MAX_MESSAGE_LEN + 1];
    let _ = publisher.send(Message::Binary(Bytes::from(huge))).await;
    // O servidor encerra a conexão (limite do WebSocket).
    let closed = loop {
        match recv(&mut publisher).await {
            None => break true,
            Some((h, _)) if h.kind == MessageType::Pong => continue,
            Some(_) => continue,
        }
    };
    assert!(closed);
}

#[tokio::test]
async fn room_limits_and_invites() {
    let (addr, _) = start(|c| c.max_viewers_per_room = 5).await;
    let room = create_room(addr, json!({"name": "pequena", "maxViewers": 1})).await;
    let t1 = viewer_token(addr, &room).await;
    let t2 = viewer_token(addr, &room).await;
    let _v1 = auth(addr, &t1, ConnectRole::Viewer).await;
    let mut v2 = connect(addr).await;
    send(&mut v2, auth_msg(&t2, ConnectRole::Viewer)).await;
    expect_error(&mut v2, ErrorCode::RoomFull).await;

    // Convite de uso único.
    let (_, inv) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/invites", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    let join_path = format!("/api/rooms/{}/join", room.id);
    for (code, expected) in [
        (inv["code"].clone(), 200),
        (inv["code"].clone(), 403),
        (json!("codigo-inventado"), 403),
    ] {
        let (s, _) = http(
            addr,
            "POST",
            &join_path,
            None,
            Some(json!({"invite": code})),
        )
        .await;
        assert_eq!(s, expected);
    }

    // Nome inválido.
    let (s, _) = http(
        addr,
        "POST",
        "/api/rooms",
        None,
        Some(json!({"name": "  "})),
    )
    .await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn create_room_key_and_rate_limit() {
    let (addr, _) = start(|c| {
        c.create_room_key = Some("chave-do-servidor".into());
        c.api_burst = 3.0;
        c.api_per_sec = 0.0;
    })
    .await;
    let body = || Some(json!({"name": "x"}));
    assert_eq!(http(addr, "POST", "/api/rooms", None, body()).await.0, 401);
    assert_eq!(
        http(addr, "POST", "/api/rooms", Some("errada"), body())
            .await
            .0,
        401
    );
    assert_eq!(
        http(
            addr,
            "POST",
            "/api/rooms",
            Some("chave-do-servidor"),
            body()
        )
        .await
        .0,
        201
    );
    assert_eq!(
        http(
            addr,
            "POST",
            "/api/rooms",
            Some("chave-do-servidor"),
            body()
        )
        .await
        .0,
        429
    );
}

#[tokio::test]
async fn slow_viewer_does_not_stall_publisher_or_other_viewers() {
    let (addr, relay) = start(|c| {
        c.viewer_queue = 64;
        c.max_publisher_kbps = 10_000_000;
        c.stale_after = Duration::from_secs(5);
        c.send_timeout = Duration::from_secs(1);
    })
    .await;
    let room = create_room(addr, json!({"name": "lenta"})).await;
    let mut publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    send(&mut publisher, video_init()).await;

    let slow_token = viewer_token(addr, &room).await;
    let _slow = auth(addr, &slow_token, ConnectRole::Viewer).await; // nunca lê
    let fast_token = viewer_token(addr, &room).await;
    let mut fast = auth(addr, &fast_token, ConnectRole::Viewer).await;

    const FRAMES: u32 = 300;
    let reader = tokio::spawn(async move {
        let mut n = 0;
        while n < FRAMES {
            let (h, _) = recv_kind(&mut fast)
                .await
                .expect("espectador rápido recebe tudo");
            if h.kind == MessageType::VideoFrame {
                n += 1;
            }
        }
        n
    });
    let started = std::time::Instant::now();
    let payload = vec![7u8; 64 * 1024];
    for i in 0..FRAMES {
        send(
            &mut publisher,
            media(MessageType::VideoFrame, i, i % 30 == 0, &payload),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(4)).await;
    }
    let publish_time = started.elapsed();
    assert_eq!(
        tokio::time::timeout(WAIT, reader).await.unwrap().unwrap(),
        FRAMES
    );
    assert!(
        publish_time < Duration::from_secs(5),
        "transmissor não foi bloqueado: {publish_time:?}"
    );
    // O espectador lento fica preso no envio; ao estourar `send_timeout` ele é
    // desconectado (ou perde mensagens antigas), sem afetar os demais.
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        let m = relay.metrics_snapshot();
        if m.slow_viewers_disconnected > 0 || m.lagged_messages > 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "espectador lento não foi tratado"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn publisher_reconnects_within_grace_period() {
    let (addr, _) = start(|c| c.publisher_grace = Duration::from_secs(10)).await;
    let room = create_room(addr, json!({"name": "queda"})).await;
    let token = viewer_token(addr, &room).await;
    let mut viewer = auth(addr, &token, ConnectRole::Viewer).await;

    let init = vellcast_protocol::messages::VideoInit {
        codec: "avc1.64002A".into(),
        coded_width: 1920,
        coded_height: 1080,
        framerate: 60,
    };
    let url = format!("ws://{addr}/ws");
    let (first, _events) = Publisher::connect(PublisherConfig::new(&url, &room.publisher));
    first.set_video_init(&init);
    let (h, _) = recv_kind(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::VideoInit);
    assert_eq!(h.flags & flags::DISCONTINUITY, 0);
    first.send_video(b"key-1", 0, true);
    let (h, _) = recv_kind(&mut viewer).await.unwrap();
    assert!(h.is_keyframe());

    // Queda sem StreamEnd: a sala entra em pausa.
    first.abort();
    let (h, _) = recv_kind(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::StreamPaused);

    // Reconexão com o mesmo token: configuração reenviada com DISCONTINUITY.
    let (second, _events2) = Publisher::connect(PublisherConfig::new(&url, &room.publisher));
    second.set_video_init(&init);
    let (h, _) = recv_kind(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::VideoInit);
    second.send_video(b"key-2", 16_666, true);
    let (h, d) = recv_kind(&mut viewer).await.unwrap();
    assert!(h.is_keyframe());
    assert_eq!(&d[vellcast_protocol::HEADER_LEN..], b"key-2");

    // Encerramento normal: espectadores recebem StreamEnd.
    second.finish(WAIT).await;
    let (h, _) = recv_kind(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::StreamEnd);
}

#[tokio::test]
async fn transport_resends_init_with_discontinuity_after_relay_drop() {
    let (addr, relay) = start(|c| {
        c.publisher_grace = Duration::from_secs(10);
        c.client_idle_timeout = Duration::from_millis(400);
    })
    .await;
    let room = create_room(addr, json!({"name": "reconexao"})).await;
    let token = viewer_token(addr, &room).await;
    let viewer = auth(addr, &token, ConnectRole::Viewer).await;
    let mut cfg = PublisherConfig::new(format!("ws://{addr}/ws"), &room.publisher);
    // Pings mais espaçados que o timeout do relay: ele derruba a conexão e o cliente reconecta.
    cfg.ping_interval = Duration::from_secs(60);
    cfg.backoff = vec![Duration::from_millis(50)];
    let (publisher, _events) = Publisher::connect(cfg);
    publisher.set_video_init(&vellcast_protocol::messages::VideoInit {
        codec: "avc1.42E01F".into(),
        coded_width: 1280,
        coded_height: 720,
        framerate: 30,
    });
    // O espectador envia Ping periodicamente para não cair por inatividade.
    let (mut vsink, mut vstream) = viewer.split();
    let pinger = tokio::spawn(async move {
        loop {
            let ping = encode(&Header::new(MessageType::Ping, 0), &[0; 8]).unwrap();
            if vsink.send(Message::Binary(ping)).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    let mut kinds = Vec::new();
    let resumed = loop {
        let msg = tokio::time::timeout(WAIT, vstream.next()).await.unwrap();
        let Some(Ok(Message::Binary(d))) = msg else {
            panic!("conexão do espectador caiu")
        };
        let (h, _) = decode(&d, MAX_MESSAGE_LEN).unwrap();
        if matches!(h.kind, MessageType::Pong | MessageType::Presence) {
            continue;
        }
        kinds.push(h.kind);
        if h.kind == MessageType::VideoInit && kinds.contains(&MessageType::StreamPaused) {
            break h;
        }
    };
    pinger.abort();
    assert_eq!(kinds[0], MessageType::VideoInit);
    assert_ne!(resumed.flags & flags::DISCONTINUITY, 0);
    assert!(publisher.stats().reconnects >= 1);
    assert_eq!(relay.metrics_snapshot().rooms_live, 1);
    publisher.abort();
}

#[tokio::test]
async fn grace_expiry_and_owner_close() {
    let (addr, relay) = start(|c| c.publisher_grace = Duration::from_millis(200)).await;
    let room = create_room(addr, json!({"name": "fim"})).await;
    let token = viewer_token(addr, &room).await;
    let mut viewer = auth(addr, &token, ConnectRole::Viewer).await;
    let publisher = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    drop(publisher);
    assert_eq!(
        recv_kind(&mut viewer).await.unwrap().0.kind,
        MessageType::StreamPaused
    );
    assert_eq!(
        recv_kind(&mut viewer).await.unwrap().0.kind,
        MessageType::StreamEnd
    );

    let (s, v) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(v["state"], "idle");

    let (s, _) = http(
        addr,
        "DELETE",
        &format!("/api/rooms/{}", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 204);
    // Espectador é desconectado com StreamEnd e/ou erro RoomClosed.
    loop {
        match recv_kind(&mut viewer).await {
            None => break,
            Some((h, d)) if h.kind == MessageType::Error => {
                assert_eq!(error_of(&d).unwrap().code, ErrorCode::RoomClosed);
            }
            Some(_) => {}
        }
    }
    assert_eq!(relay.metrics_snapshot().rooms, 0);
    let (s, _) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(s, 404);
}
