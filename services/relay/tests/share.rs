//! Compartilhamento por participantes (issue #4): pedido explícito de
//! transmissão, troca de quem compartilha, vaga única, liberação da vaga e
//! autorização. Relay real em porta efêmera, clientes WebSocket e HTTP reais.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use vellcast_protocol::messages::{
    AuthRequest, ConnectRole, ErrorBody, ErrorCode, ParticipantRole, Presence, ShareInfo,
};
use vellcast_protocol::{HEADER_LEN, Header, MAX_MESSAGE_LEN, MessageType, decode, encode, flags};
use vellcast_relay::config::Config;
use vellcast_relay::store::{Invite, hash_invite};
use vellcast_relay::token::Role;
use vellcast_relay::{Relay, unix_now};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

const SECRET: &[u8] = b"share-test-secret-share-test-secre";
const WAIT: Duration = Duration::from_secs(5);

/// Log de todos os testes deste binário, para verificar que segredos não vazam.
fn captured_logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::clone(&buf);
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || LogWriter(Arc::clone(&writer)))
            .try_init();
        buf
    })
}

struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn start(tweak: impl FnOnce(&mut Config)) -> (SocketAddr, Arc<Relay>) {
    captured_logs();
    let mut config = Config::with_secret(SECRET.to_vec());
    config.cleanup_interval = Duration::from_millis(50);
    tweak(&mut config);
    let relay = Relay::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(vellcast_relay::serve(Arc::clone(&relay), listener));
    (addr, relay)
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
        Some(json!({"name": "Lobby"})),
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

async fn join(addr: SocketAddr, room: &str, code: &str) -> (u16, Value) {
    http(
        addr,
        "POST",
        &format!("/api/rooms/{room}/join"),
        None,
        Some(json!({ "invite": code })),
    )
    .await
}

/// Participante: entra com um convite novo e devolve o token de espectador.
async fn participant(addr: SocketAddr, room: &Room) -> String {
    let code = invite(addr, room, 1).await;
    let (s, joined) = join(addr, &room.id, &code).await;
    assert_eq!(s, 200, "{joined}");
    joined["viewerToken"].as_str().unwrap().into()
}

async fn share(addr: SocketAddr, room: &str, token: Option<&str>, body: Value) -> (u16, Value) {
    http(
        addr,
        "POST",
        &format!("/api/rooms/{room}/share"),
        token,
        Some(body),
    )
    .await
}

async fn share_token(addr: SocketAddr, room: &str, token: &str, name: &str) -> String {
    let (s, v) = share(addr, room, Some(token), json!({ "name": name })).await;
    assert_eq!(s, 200, "{v}");
    v["publisherToken"].as_str().unwrap().into()
}

async fn connect(addr: SocketAddr, token: &str, role: ConnectRole) -> (Ws, Header, Bytes) {
    let mut ws = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap()
        .0;
    let body = serde_json::to_vec(&AuthRequest {
        token: token.into(),
        role,
    })
    .unwrap();
    send(
        &mut ws,
        encode(&Header::new(MessageType::Auth, 0), &body).unwrap(),
    )
    .await;
    let (h, d) = recv(&mut ws).await.expect("resposta de auth");
    (ws, h, d)
}

async fn auth(addr: SocketAddr, token: &str, role: ConnectRole) -> Ws {
    let (ws, h, d) = connect(addr, token, role).await;
    assert_eq!(h.kind, MessageType::AuthOk, "{:?}", error_of(&d));
    ws
}

async fn send(ws: &mut Ws, data: Bytes) {
    ws.send(Message::Binary(data)).await.unwrap();
}

/// Próxima mensagem que não seja Pong nem Presence (coberta em `tests/presence.rs`);
/// `None` se a conexão fechou.
async fn recv(ws: &mut Ws) -> Option<(Header, Bytes)> {
    loop {
        match tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("timeout esperando mensagem")
        {
            Some(Ok(Message::Binary(d))) => {
                let (h, _) = decode(&d, MAX_MESSAGE_LEN).unwrap();
                if matches!(h.kind, MessageType::Pong | MessageType::Presence) {
                    continue;
                }
                return Some((h, d));
            }
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return None,
            Some(Ok(_)) => continue,
        }
    }
}

fn error_of(data: &Bytes) -> Option<ErrorBody> {
    serde_json::from_slice(&data[HEADER_LEN..]).ok()
}

fn share_of(data: &Bytes) -> ShareInfo {
    serde_json::from_slice(&data[HEADER_LEN..]).unwrap()
}

async fn expect_error(ws: &mut Ws, code: ErrorCode) {
    let (h, d) = recv(ws).await.expect("mensagem de erro");
    assert_eq!(h.kind, MessageType::Error, "{h:?}");
    assert_eq!(error_of(&d).unwrap().code, code);
}

fn video_init(width: u32) -> Bytes {
    let body =
        json!({"codec": "avc1.64002A", "codedWidth": width, "codedHeight": 720, "framerate": 60});
    encode(
        &Header::new(MessageType::VideoInit, 0),
        body.to_string().as_bytes(),
    )
    .unwrap()
}

fn frame(seq: u32, keyframe: bool, payload: &[u8]) -> Bytes {
    let mut h = Header::new(MessageType::VideoFrame, 0);
    h.sequence = seq;
    h.pts_us = u64::from(seq) * 16_666;
    if keyframe {
        h.flags = flags::KEYFRAME;
    }
    encode(&h, payload).unwrap()
}

async fn room_view(addr: SocketAddr, room: &Room) -> Value {
    let (s, v) = http(
        addr,
        "GET",
        &format!("/api/rooms/{}", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 200, "{v}");
    v
}

#[tokio::test]
async fn participant_shares_and_hands_off_to_another() {
    let (addr, relay) = start(|_| {}).await;
    let room = create_room(addr).await;
    let ana = participant(addr, &room).await;
    let bia = participant(addr, &room).await;
    let watcher = participant(addr, &room).await;
    let mut viewer = auth(addr, &watcher, ConnectRole::Viewer).await;

    // Ana pede para compartilhar e assume a sala vazia.
    let ana_pub = share_token(addr, &room.id, &ana, "Ana").await;
    let mut ana_ws = auth(addr, &ana_pub, ConnectRole::Publisher).await;
    let (h, d) = recv(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::ShareChanged);
    assert_eq!(
        share_of(&d),
        ShareInfo {
            name: Some("Ana".into()),
            handoff: false
        }
    );
    send(&mut ana_ws, video_init(1280)).await;
    send(&mut ana_ws, frame(0, true, b"ana-key")).await;
    send(&mut ana_ws, frame(1, false, b"ana-delta")).await;
    assert_eq!(
        recv(&mut viewer).await.unwrap().0.kind,
        MessageType::VideoInit
    );
    assert_eq!(
        &recv(&mut viewer).await.unwrap().1[HEADER_LEN..],
        b"ana-key"
    );
    assert_eq!(
        &recv(&mut viewer).await.unwrap().1[HEADER_LEN..],
        b"ana-delta"
    );
    let v = room_view(addr, &room).await;
    assert_eq!(
        (v["state"].as_str(), v["sharerName"].as_str()),
        (Some("live"), Some("Ana"))
    );

    // Bia pede para compartilhar: troca explícita.
    let bia_pub = share_token(addr, &room.id, &bia, "Bia").await;
    let mut bia_ws = auth(addr, &bia_pub, ConnectRole::Publisher).await;

    // Ana é avisada de quem assumiu e recebe o erro fatal para clientes antigos.
    let (h, d) = recv(&mut ana_ws).await.unwrap();
    assert_eq!(h.kind, MessageType::PublisherReplaced);
    assert_eq!(share_of(&d).name.as_deref(), Some("Bia"));
    expect_error(&mut ana_ws, ErrorCode::PublisherExists).await;

    // Espectador: aviso de troca, depois só o fluxo novo, a partir do keyframe.
    let (h, d) = recv(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::ShareChanged);
    assert_eq!(
        share_of(&d),
        ShareInfo {
            name: Some("Bia".into()),
            handoff: true
        }
    );
    send(&mut bia_ws, frame(0, false, b"bia-delta-orfao")).await;
    send(&mut bia_ws, video_init(1920)).await;
    send(&mut bia_ws, frame(1, true, b"bia-key")).await;
    send(&mut bia_ws, frame(2, false, b"bia-delta")).await;
    let (h, d) = recv(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::VideoInit);
    assert!(
        String::from_utf8_lossy(&d).contains("1920"),
        "configuração nova"
    );
    assert_eq!(
        &recv(&mut viewer).await.unwrap().1[HEADER_LEN..],
        b"bia-key"
    );
    assert_eq!(
        &recv(&mut viewer).await.unwrap().1[HEADER_LEN..],
        b"bia-delta"
    );

    // Quem entra agora recebe só a configuração e o GOP de Bia.
    let late = participant(addr, &room).await;
    let mut late_ws = auth(addr, &late, ConnectRole::Viewer).await;
    let (h, d) = recv(&mut late_ws).await.unwrap();
    assert_eq!(h.kind, MessageType::ShareChanged);
    assert_eq!(share_of(&d).name.as_deref(), Some("Bia"));
    let (_, d) = recv(&mut late_ws).await.unwrap();
    assert!(String::from_utf8_lossy(&d).contains("1920"));
    assert_eq!(
        &recv(&mut late_ws).await.unwrap().1[HEADER_LEN..],
        b"bia-key"
    );

    // A reconexão com o token de Ana (já usado) não toma a vaga de volta.
    let (mut again, h, d) = connect(addr, &ana_pub, ConnectRole::Publisher).await;
    assert_eq!(h.kind, MessageType::Error);
    assert_eq!(error_of(&d).unwrap().code, ErrorCode::PublisherExists);
    assert!(recv(&mut again).await.is_none());
    let v = room_view(addr, &room).await;
    assert_eq!(v["sharerName"].as_str(), Some("Bia"));
    assert_eq!(relay.metrics_snapshot().publisher_handoffs, 1);
    assert_eq!(relay.metrics_snapshot().rooms_live, 1);
}

#[tokio::test]
async fn only_one_publisher_without_explicit_share() {
    let (addr, _) = start(|_| {}).await;
    let room = create_room(addr).await;
    let ana = participant(addr, &room).await;
    let ana_pub = share_token(addr, &room.id, &ana, "Ana").await;
    let _ana_ws = auth(addr, &ana_pub, ConnectRole::Publisher).await;

    // O token de transmissor comum (sem pedido de compartilhamento) não derruba ninguém.
    let (_, h, d) = connect(addr, &room.publisher, ConnectRole::Publisher).await;
    assert_eq!(h.kind, MessageType::Error);
    assert_eq!(error_of(&d).unwrap().code, ErrorCode::PublisherExists);
    // O dono também pode pedir a vaga explicitamente.
    let owner_pub = share_token(addr, &room.id, &room.owner, "Dono").await;
    let _owner_ws = auth(addr, &owner_pub, ConnectRole::Publisher).await;
    let v = room_view(addr, &room).await;
    assert_eq!(v["sharerName"].as_str(), Some("Dono"));
}

#[tokio::test]
async fn stopping_or_disconnecting_releases_the_slot() {
    let (addr, _) = start(|c| c.publisher_grace = Duration::from_secs(30)).await;
    let room = create_room(addr).await;
    let ana = participant(addr, &room).await;
    let bia = participant(addr, &room).await;
    let watcher = participant(addr, &room).await;
    let mut viewer = auth(addr, &watcher, ConnectRole::Viewer).await;

    // Parar (StreamEnd) libera a vaga: a sala volta a ficar ociosa.
    let mut ana_ws = auth(
        addr,
        &share_token(addr, &room.id, &ana, "Ana").await,
        ConnectRole::Publisher,
    )
    .await;
    assert_eq!(
        recv(&mut viewer).await.unwrap().0.kind,
        MessageType::ShareChanged
    );
    send(
        &mut ana_ws,
        encode(&Header::new(MessageType::StreamEnd, 0), &[]).unwrap(),
    )
    .await;
    assert_eq!(
        recv(&mut viewer).await.unwrap().0.kind,
        MessageType::StreamEnd
    );
    let v = room_view(addr, &room).await;
    assert_eq!(v["state"].as_str(), Some("idle"));
    assert!(v.get("sharerName").is_none());

    // Queda sem StreamEnd: a vaga fica livre na hora (sem esperar a tolerância).
    let bia_ws = auth(
        addr,
        &share_token(addr, &room.id, &bia, "Bia").await,
        ConnectRole::Publisher,
    )
    .await;
    assert_eq!(
        recv(&mut viewer).await.unwrap().0.kind,
        MessageType::ShareChanged
    );
    drop(bia_ws);
    assert_eq!(
        recv(&mut viewer).await.unwrap().0.kind,
        MessageType::StreamPaused
    );
    // Até um token comum pode transmitir agora, sem troca.
    let _owner_ws = auth(addr, &room.publisher, ConnectRole::Publisher).await;
    let (h, d) = recv(&mut viewer).await.unwrap();
    assert_eq!(h.kind, MessageType::ShareChanged);
    assert_eq!(share_of(&d), ShareInfo::default());
    assert_eq!(room_view(addr, &room).await["state"].as_str(), Some("live"));
}

#[tokio::test]
async fn share_requires_a_valid_invite_membership() {
    let (addr, relay) = start(|_| {}).await;
    let room = create_room(addr).await;
    let other = create_room(addr).await;
    let id = room.id.as_str();

    // Sem token, token forjado ou de outra sala.
    assert_eq!(share(addr, id, None, json!({})).await.0, 401);
    assert_eq!(
        share(addr, id, Some("v1.forjado.assinatura"), json!({}))
            .await
            .0,
        401
    );
    let stranger = participant(addr, &other).await;
    assert_eq!(share(addr, id, Some(&stranger), json!({})).await.0, 401);
    // Token de transmissor não pede vaga; espectador sem convite (ex.: Discord) também não.
    assert_eq!(
        share(addr, id, Some(&room.publisher), json!({})).await.0,
        403
    );
    let (no_member, _) = relay
        .signer
        .issue(id, Role::Viewer, Duration::from_secs(60), unix_now());
    assert_eq!(share(addr, id, Some(&no_member), json!({})).await.0, 403);
    // Nome inválido (mesmas regras do nome do convidado).
    let ana = participant(addr, &room).await;
    assert_eq!(
        share(addr, id, Some(&ana), json!({"name": "\u{7}\u{1b}"}))
            .await
            .0,
        400
    );
    assert_eq!(
        share(addr, id, Some(&ana), json!({"name": "x".repeat(33)}))
            .await
            .0,
        400
    );
    // Sem nome também vale (o nome é opcional).
    assert_eq!(share(addr, id, Some(&ana), json!({})).await.0, 200);

    // Convites malformados, esgotados e expirados não dão acesso nenhum.
    let code = invite(addr, &room, 1).await;
    assert_eq!(join(addr, id, &code).await.0, 200);
    let (s, v) = join(addr, id, &code).await;
    assert_eq!(s, 403, "esgotado");
    assert!(v.get("viewerToken").is_none());
    assert_eq!(join(addr, id, "nao-existe").await.0, 403);
    assert_eq!(join(addr, id, &"x".repeat(200)).await.0, 400);
    relay
        .store
        .add_invite(
            id,
            hash_invite("convite-vencido"),
            Invite {
                remaining_uses: 5,
                expires_at: unix_now() - 1,
            },
        )
        .unwrap();
    assert_eq!(join(addr, id, "convite-vencido").await.0, 403, "expirado");
    // Token de espectador vencido também não assiste nem compartilha.
    let (expired, _) = relay
        .signer
        .issue_with(id, Role::Viewer, Duration::ZERO, unix_now(), |c| {
            c.member = true;
        });
    assert_eq!(share(addr, id, Some(&expired), json!({})).await.0, 401);
    let (_, h, d) = connect(addr, &expired, ConnectRole::Viewer).await;
    assert_eq!(h.kind, MessageType::Error);
    assert_eq!(error_of(&d).unwrap().code, ErrorCode::Unauthorized);
    // Espectador não transmite com o próprio token de espectador.
    let (_, h, d) = connect(addr, &ana, ConnectRole::Publisher).await;
    assert_eq!(h.kind, MessageType::Error);
    assert_eq!(error_of(&d).unwrap().code, ErrorCode::Forbidden);
}

#[tokio::test]
async fn revoking_participants_stops_their_share() {
    let (addr, _) = start(|_| {}).await;
    let room = create_room(addr).await;
    let ana = participant(addr, &room).await;
    let bia = participant(addr, &room).await;
    let ana_pub = share_token(addr, &room.id, &ana, "Ana").await;
    let bia_pub = share_token(addr, &room.id, &bia, "Bia").await;

    // Revogação individual do participante (pelo token de espectador) vale
    // também para o compartilhamento derivado dele.
    let (s, _) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.owner),
        Some(json!({ "token": bia })),
    )
    .await;
    assert_eq!(s, 200);
    let (_, h, d) = connect(addr, &bia_pub, ConnectRole::Publisher).await;
    assert_eq!(h.kind, MessageType::Error);
    assert_eq!(error_of(&d).unwrap().code, ErrorCode::Unauthorized);

    // O dono revoga todos os espectadores: quem estava compartilhando cai junto.
    let mut ana_ws = auth(addr, &ana_pub, ConnectRole::Publisher).await;
    let (s, v) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/revoke", room.id),
        Some(&room.owner),
        Some(json!({ "allViewers": true })),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    expect_error(&mut ana_ws, ErrorCode::Unauthorized).await;
    assert_eq!(share(addr, &room.id, Some(&ana), json!({})).await.0, 401);
    let (_, h, _) = connect(addr, &ana_pub, ConnectRole::Publisher).await;
    assert_eq!(h.kind, MessageType::Error);

    // Encerrar a sala continua funcionando com alguém compartilhando.
    let owner_pub = share_token(addr, &room.id, &room.owner, "Dono").await;
    let mut owner_ws = auth(addr, &owner_pub, ConnectRole::Publisher).await;
    let (s, _) = http(
        addr,
        "DELETE",
        &format!("/api/rooms/{}", room.id),
        Some(&room.owner),
        None,
    )
    .await;
    assert_eq!(s, 204);
    expect_error(&mut owner_ws, ErrorCode::RoomClosed).await;
}

#[tokio::test]
async fn logs_never_contain_invites_tokens_or_names() {
    let (addr, _) = start(|_| {}).await;
    let room = create_room(addr).await;
    let code = invite(addr, &room, 2).await;
    let (_, joined) = join(addr, &room.id, &code).await;
    let viewer = joined["viewerToken"].as_str().unwrap().to_owned();
    let name = "NomeSecretoDoTeste";
    let publisher = share_token(addr, &room.id, &viewer, name).await;
    let mut ws = auth(addr, &publisher, ConnectRole::Publisher).await;
    send(
        &mut ws,
        encode(&Header::new(MessageType::StreamEnd, 0), &[]).unwrap(),
    )
    .await;
    let _ = recv(&mut ws).await;
    let _ = share(addr, &room.id, Some("v1.forjado.assinatura"), json!({})).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let logs = String::from_utf8_lossy(&captured_logs().lock().unwrap()).into_owned();
    assert!(
        logs.contains("token de compartilhamento emitido"),
        "o log foi capturado"
    );
    for secret in [&code, &viewer, &publisher, &room.owner, &room.publisher] {
        assert!(!logs.contains(secret.as_str()), "segredo no log");
    }
    assert!(!logs.contains(name), "nome do participante no log");
}

/// Próxima lista de participantes recebida, ignorando as outras mensagens.
async fn next_presence(ws: &mut Ws) -> Presence {
    loop {
        match tokio::time::timeout(WAIT, ws.next())
            .await
            .expect("timeout esperando Presence")
        {
            Some(Ok(Message::Binary(d))) => {
                let (h, _) = decode(&d, MAX_MESSAGE_LEN).unwrap();
                if h.kind == MessageType::Presence {
                    return serde_json::from_slice(&d[HEADER_LEN..]).unwrap();
                }
            }
            Some(Ok(_)) => continue,
            other => panic!("conexão fechou: {other:?}"),
        }
    }
}

/// Lê listas até a condição valer (a lista é completa a cada mudança).
async fn presence_until(ws: &mut Ws, ok: impl Fn(&Presence) -> bool) -> Presence {
    loop {
        let p = next_presence(ws).await;
        if ok(&p) {
            return p;
        }
    }
}

fn sharers(p: &Presence) -> Vec<Option<String>> {
    p.participants
        .iter()
        .filter(|x| x.role == ParticipantRole::Publisher)
        .map(|x| x.name.clone())
        .collect()
}

async fn named_participant(addr: SocketAddr, room: &Room, name: &str) -> String {
    let code = invite(addr, room, 1).await;
    let (s, v) = http(
        addr,
        "POST",
        &format!("/api/rooms/{}/join", room.id),
        None,
        Some(json!({ "invite": code, "name": name })),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    v["viewerToken"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn guest_name_is_the_default_sharer_name_and_presence_follows_handoff() {
    let (addr, _) = start(|_| {}).await;
    let room = create_room(addr).await;
    let ana = named_participant(addr, &room, "Ana").await;
    let bia = named_participant(addr, &room, "Bia").await;
    let watcher = participant(addr, &room).await;
    let mut viewer = auth(addr, &watcher, ConnectRole::Viewer).await;

    // Sem `name` no pedido, vale o nome assinado no token do participante.
    let (s, v) = share(addr, &room.id, Some(&ana), json!({})).await;
    assert_eq!(s, 200, "{v}");
    let ana_pub = v["publisherToken"].as_str().unwrap().to_owned();
    let _ana_ws = auth(addr, &ana_pub, ConnectRole::Publisher).await;
    let p = presence_until(&mut viewer, |p| !sharers(p).is_empty()).await;
    assert_eq!(sharers(&p), vec![Some("Ana".to_owned())]);
    assert_eq!(
        room_view(addr, &room).await["sharerName"].as_str(),
        Some("Ana")
    );

    // Troca: a lista passa a mostrar só Bia transmitindo.
    let bia_pub = share_token(addr, &room.id, &bia, "Bia").await;
    let _bia_ws = auth(addr, &bia_pub, ConnectRole::Publisher).await;
    presence_until(&mut viewer, |p| sharers(p) == vec![Some("Bia".to_owned())]).await;
}
