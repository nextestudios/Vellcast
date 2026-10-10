//! Conexões WebSocket de transmissores e espectadores.
//!
//! A primeira mensagem precisa ser `Auth` (dentro de `auth_timeout`).
//! O token nunca aparece na URL nem em logs.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast::error::{RecvError, TryRecvError};
use vellcast_protocol::messages::{
    AudioInit, AuthOk, AuthProfile, AuthRequest, ConnectRole, ErrorCode, ParticipantRole,
    VideoInit, encode_json, error_message, sanitize_participant_name,
};
use vellcast_protocol::{
    Header, MAX_AUTH_MESSAGE_LEN, MAX_CLIENT_MESSAGE_LEN, MAX_MESSAGE_LEN, MessageType, decode,
    encode, flags,
};

use crate::ratelimit::TokenBucket;
use crate::room::{LiveRoom, Relayed, RoomState};
use crate::token::{Claims, Role};
use crate::{Relay, metrics, random_id, unix_now};

type Sink = SplitSink<WebSocket, Message>;
type Source = SplitStream<WebSocket>;

/// Limite do texto de configuração (codec string etc.).
const MAX_INIT_LEN: usize = 1024;
/// Mensagens enviadas por espectador em um único `flush`.
const MAX_BATCH: usize = 64;

/// Decide se uma mensagem distribuída segue para um espectador: descarta
/// quadros velhos e, após perda, tudo até o próximo keyframe.
struct Gate<'a> {
    waiting_keyframe: &'a mut bool,
    stale_after: std::time::Duration,
}

impl Gate<'_> {
    fn admit(&mut self, msg: Relayed, relay: &Relay, room: &LiveRoom) -> Option<Bytes> {
        match msg.kind {
            MessageType::VideoFrame => {
                if msg.received_at.elapsed() > self.stale_after {
                    metrics::inc(&relay.metrics.stale_frames_dropped);
                    *self.waiting_keyframe = true;
                    room.request_keyframe(Instant::now());
                    return None;
                }
                if *self.waiting_keyframe {
                    if !msg.keyframe {
                        return None;
                    }
                    *self.waiting_keyframe = false;
                }
            }
            MessageType::StreamPaused | MessageType::StreamEnd => *self.waiting_keyframe = true,
            _ => {}
        }
        Some(msg.data)
    }
}

pub async fn upgrade(
    ws: WebSocketUpgrade,
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Response {
    if !relay.ws_limiter.check(relay.client_ip(peer, &headers)) {
        metrics::inc(&relay.metrics.rate_limited);
        return (StatusCode::TOO_MANY_REQUESTS, "muitas conexões").into_response();
    }
    ws.max_message_size(MAX_MESSAGE_LEN)
        .max_frame_size(MAX_MESSAGE_LEN)
        .on_upgrade(move |socket| handle(socket, relay))
}

async fn send_error(sink: &mut Sink, code: ErrorCode, message: &str) {
    let _ = sink
        .send(Message::Binary(error_message(code, message)))
        .await;
    let _ = sink
        .send(Message::Close(Some(CloseFrame {
            code: 4000,
            reason: "".into(),
        })))
        .await;
}

struct Authed {
    role: ConnectRole,
    room: Arc<LiveRoom>,
    nonce: String,
    claims: Claims,
    /// Nome na lista de participantes: do token (espectador) ou do `Auth` (transmissor).
    display_name: Option<String>,
}

/// Nome de exibição de uma conexão. Espectadores usam o nome assinado no
/// token (escolhido ao entrar com o convite) e não podem trocá-lo pelo `Auth`.
fn display_name(role: ConnectRole, claims: &Claims, payload: &[u8]) -> Option<String> {
    let from_auth = || {
        serde_json::from_slice::<AuthProfile>(payload)
            .ok()?
            .name
            .as_deref()
            .and_then(sanitize_participant_name)
    };
    match role {
        ConnectRole::Viewer => claims.name.clone(),
        ConnectRole::Publisher => from_auth().or_else(|| claims.name.clone()),
    }
}

async fn authenticate(
    relay: &Relay,
    source: &mut Source,
) -> Result<Authed, (ErrorCode, &'static str)> {
    let first = tokio::time::timeout(relay.config.auth_timeout, source.next())
        .await
        .map_err(|_| (ErrorCode::AuthTimeout, "Auth não recebido a tempo"))?;
    let Some(Ok(Message::Binary(data))) = first else {
        return Err((
            ErrorCode::ProtocolError,
            "a primeira mensagem deve ser Auth",
        ));
    };
    let (header, payload) = decode(&data, MAX_AUTH_MESSAGE_LEN)
        .map_err(|_| (ErrorCode::ProtocolError, "mensagem Auth inválida"))?;
    if header.kind != MessageType::Auth {
        return Err((
            ErrorCode::ProtocolError,
            "a primeira mensagem deve ser Auth",
        ));
    }
    let req: AuthRequest = serde_json::from_slice(&payload)
        .map_err(|_| (ErrorCode::ProtocolError, "Auth malformado"))?;
    let claims = relay
        .signer
        .verify(&req.token, unix_now())
        .map_err(|_| (ErrorCode::Unauthorized, "token inválido ou expirado"))?;
    let allowed = match req.role {
        ConnectRole::Publisher => matches!(claims.role, Role::Publisher | Role::Owner),
        ConnectRole::Viewer => matches!(claims.role, Role::Viewer | Role::Owner),
    };
    if !allowed {
        return Err((ErrorCode::Forbidden, "token sem permissão para este papel"));
    }
    if relay.is_revoked(&claims) {
        return Err((ErrorCode::Unauthorized, "token revogado"));
    }
    let room = relay
        .room(&claims.room)
        .ok_or((ErrorCode::RoomNotFound, "sala não encontrada"))?;
    if room.state() == RoomState::Closed {
        return Err((ErrorCode::RoomClosed, "sala encerrada"));
    }
    Ok(Authed {
        role: req.role,
        room,
        nonce: claims.nonce.clone(),
        display_name: display_name(req.role, &claims, &payload),
        claims,
    })
}

async fn handle(socket: WebSocket, relay: Arc<Relay>) {
    let (mut sink, mut source) = socket.split();
    let authed = match authenticate(&relay, &mut source).await {
        Ok(a) => a,
        Err((code, msg)) => {
            metrics::inc(&relay.metrics.auth_failures);
            tracing::debug!(?code, "autenticação recusada");
            send_error(&mut sink, code, msg).await;
            return;
        }
    };
    let session_id = random_id(8);
    match authed.role {
        ConnectRole::Publisher => {
            metrics::inc(&relay.metrics.publisher_sessions_total);
            run_publisher(&relay, authed, &session_id, sink, source).await;
        }
        ConnectRole::Viewer => {
            if !authed.room.reserve_viewer() {
                send_error(&mut sink, ErrorCode::RoomFull, "sala cheia").await;
                return;
            }
            metrics::inc(&relay.metrics.viewer_sessions_total);
            let room = Arc::clone(&authed.room);
            let presence = room.join_presence(
                &authed.nonce,
                ParticipantRole::Viewer,
                authed.display_name.clone(),
            );
            run_viewer(&relay, &room, &authed.claims, &session_id, sink, source).await;
            drop(presence);
            room.release_viewer();
        }
    }
}

fn auth_ok(room: &LiveRoom, role: ConnectRole, session_id: &str) -> Option<Bytes> {
    encode_json(
        MessageType::AuthOk,
        0,
        &AuthOk {
            room_id: room.id.clone(),
            role,
            session_id: session_id.to_owned(),
            live: room.is_live(),
        },
    )
    .ok()
}

fn pong(header: &Header, payload: &[u8]) -> Option<Bytes> {
    let mut h = Header::new(MessageType::Pong, header.stream_id);
    h.sequence = header.sequence;
    h.sent_at_us = crate::now_us();
    encode(&h, payload).ok()
}

/// Valida o conteúdo das mensagens de configuração antes de repassá-las.
fn valid_init(kind: MessageType, payload: &[u8]) -> bool {
    let is_init = matches!(kind, MessageType::VideoInit | MessageType::AudioInit);
    if is_init && payload.len() > MAX_INIT_LEN {
        return false;
    }
    match kind {
        MessageType::VideoInit => serde_json::from_slice::<VideoInit>(payload)
            .is_ok_and(|v| v.codec.len() <= 64 && v.coded_width > 0 && v.coded_height > 0),
        MessageType::AudioInit => {
            serde_json::from_slice::<AudioInit>(payload).is_ok_and(|a| a.codec.len() <= 64)
        }
        _ => true,
    }
}

async fn run_publisher(
    relay: &Relay,
    authed: Authed,
    session_id: &str,
    mut sink: Sink,
    mut source: Source,
) {
    let room = authed.room;
    let claims = authed.claims;
    let mut lease = match room.attach_publisher(session_id, &authed.nonce) {
        Ok(l) => l,
        Err(code) => {
            send_error(&mut sink, code, "não foi possível transmitir nesta sala").await;
            return;
        }
    };
    // Removido da lista de participantes quando esta função termina.
    let _presence = room.join_presence(
        &authed.nonce,
        ParticipantRole::Publisher,
        authed.display_name,
    );
    tracing::info!(room = %room.id, session = %session_id, "transmissor conectado");
    if let Some(ok) = auth_ok(&room, ConnectRole::Publisher, session_id) {
        let _ = sink.send(Message::Binary(ok)).await;
    }
    let mut closed = room.closed_signal();
    let mut revocations = room.revocation_signal();
    let bytes_per_sec = relay.config.max_publisher_kbps as f64 * 1000.0 / 8.0;
    // Rajada de 2 s: keyframes são bem maiores que a média.
    let mut ingress = TokenBucket::new(bytes_per_sec * 2.0, bytes_per_sec, Instant::now());
    let idle = relay.config.client_idle_timeout;

    let graceful = loop {
        tokio::select! {
            incoming = tokio::time::timeout(idle, source.next()) => {
                let data = match incoming {
                    Ok(Some(Ok(Message::Binary(d)))) => d,
                    Ok(Some(Ok(Message::Close(_)) | Err(_))) | Ok(None) => break false,
                    Ok(Some(Ok(_))) => continue,
                    Err(_) => {
                        send_error(&mut sink, ErrorCode::IdleTimeout, "inatividade").await;
                        break false;
                    }
                };
                metrics::inc(&relay.metrics.messages_in);
                metrics::add(&relay.metrics.bytes_in, data.len() as u64);
                if !ingress.try_take(data.len() as f64, Instant::now()) {
                    send_error(&mut sink, ErrorCode::BitrateExceeded, "bitrate acima do limite do servidor").await;
                    break false;
                }
                let Ok(header) = Header::read(&data) else {
                    send_error(&mut sink, ErrorCode::ProtocolError, "mensagem inválida").await;
                    break false;
                };
                match header.kind {
                    MessageType::VideoInit | MessageType::AudioInit
                    | MessageType::VideoFrame | MessageType::AudioFrame => {
                        let payload = &data[vellcast_protocol::HEADER_LEN..];
                        if !valid_init(header.kind, payload) {
                            send_error(&mut sink, ErrorCode::ProtocolError, "configuração inválida").await;
                            break false;
                        }
                        room.publish(Relayed {
                            kind: header.kind,
                            keyframe: header.flags & flags::KEYFRAME != 0,
                            received_at: Instant::now(),
                            data,
                        });
                    }
                    MessageType::StreamEnd => break true,
                    MessageType::Ping => {
                        if let Some(p) = pong(&header, &data[vellcast_protocol::HEADER_LEN..]) {
                            let _ = sink.send(Message::Binary(p)).await;
                        }
                    }
                    _ => {}
                }
            }
            Some(()) = lease.keyframe_rx.recv() => {
                metrics::inc(&relay.metrics.keyframe_requests_forwarded);
                if let Ok(msg) = encode(&Header::new(MessageType::KeyframeRequest, 0), &[]) {
                    let _ = sink.send(Message::Binary(msg)).await;
                }
            }
            kicked = &mut lease.kicked => {
                // Ok: outra conexão com o mesmo token assumiu. Err: a vaga foi
                // removida (sala encerrada).
                let (code, msg) = if kicked.is_ok() {
                    (ErrorCode::PublisherExists, "substituído por uma nova conexão")
                } else {
                    (ErrorCode::RoomClosed, "sala encerrada")
                };
                send_error(&mut sink, code, msg).await;
                break kicked.is_err();
            }
            _ = closed.changed() => {
                send_error(&mut sink, ErrorCode::RoomClosed, "sala encerrada").await;
                break true;
            }
            _ = revocations.changed() => {
                if relay.is_revoked(&claims) {
                    send_error(&mut sink, ErrorCode::Unauthorized, "token revogado").await;
                    break true;
                }
            }
        }
    };
    room.detach_publisher(session_id, graceful);
    tracing::info!(room = %room.id, session = %session_id, graceful, "transmissor desconectado");
}

async fn run_viewer(
    relay: &Relay,
    room: &LiveRoom,
    claims: &Claims,
    session_id: &str,
    mut sink: Sink,
    mut source: Source,
) {
    let join = room.join_viewer();
    let mut rx = join.rx;
    let send_timeout = relay.config.send_timeout;
    let stale_after = relay.config.stale_after;
    let batch_writes = relay.config.batch_writes;
    // Reaproveitado entre lotes: nenhuma alocação por mensagem no caminho quente.
    let mut batch: Vec<Bytes> = Vec::with_capacity(MAX_BATCH);

    let mut presence = room.presence_signal();
    let mut initial = Vec::with_capacity(join.snapshot.len() + 3);
    initial.extend(auth_ok(room, ConnectRole::Viewer, session_id));
    if join.state == RoomState::Paused {
        initial.extend(encode(&Header::new(MessageType::StreamPaused, 0), &[]).ok());
    }
    // Lista de participantes atual (já inclui esta conexão); as mudanças
    // seguintes chegam pelo `presence.changed()` do laço.
    initial.push(presence.borrow_and_update().clone());
    initial.extend(join.snapshot);
    // Configuração + GOP em cache (até centenas de mensagens): uma única escrita.
    let initial_bytes: u64 = initial.iter().map(|m| m.len() as u64).sum();
    let write_initial = async {
        for msg in initial {
            sink.feed(Message::Binary(msg)).await?;
        }
        sink.flush().await
    };
    if !matches!(
        tokio::time::timeout(send_timeout, write_initial).await,
        Ok(Ok(()))
    ) {
        return;
    }
    metrics::add(&relay.metrics.bytes_out, initial_bytes);
    let mut waiting_keyframe = !join.has_keyframe;
    if waiting_keyframe && join.state == RoomState::Live {
        room.request_keyframe(Instant::now());
    }
    tracing::debug!(room = %room.id, session = %session_id, "espectador conectado");

    let mut closed = room.closed_signal();
    let mut revocations = room.revocation_signal();
    let idle = tokio::time::sleep(relay.config.client_idle_timeout);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            incoming = source.next() => {
                let data = match incoming {
                    Some(Ok(Message::Binary(d))) => d,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                idle.as_mut().reset(tokio::time::Instant::now() + relay.config.client_idle_timeout);
                let Ok((header, payload)) = decode(&data, MAX_CLIENT_MESSAGE_LEN) else {
                    send_error(&mut sink, ErrorCode::MessageTooLarge, "mensagem inválida ou grande demais").await;
                    break;
                };
                match header.kind {
                    MessageType::Ping => {
                        if let Some(p) = pong(&header, &payload) {
                            let _ = sink.send(Message::Binary(p)).await;
                        }
                    }
                    MessageType::KeyframeRequest => {
                        room.request_keyframe(Instant::now());
                    }
                    _ => {}
                }
            }
            outgoing = rx.recv() => {
                let first = match outgoing {
                    Ok(m) => m,
                    Err(RecvError::Lagged(n)) => {
                        metrics::add(&relay.metrics.lagged_messages, n);
                        waiting_keyframe = true;
                        room.request_keyframe(Instant::now());
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                };
                // Monta um lote com o que já está na fila: as mensagens são
                // enfileiradas com `feed` e enviadas com um único `flush`
                // (uma escrita no socket por lote em vez de uma por mensagem).
                batch.clear();
                let mut gate = Gate { waiting_keyframe: &mut waiting_keyframe, stale_after };
                if let Some(data) = gate.admit(first, relay, room) {
                    batch.push(data);
                }
                if batch_writes {
                    let mut closed_now = false;
                    while batch.len() < MAX_BATCH {
                        match rx.try_recv() {
                            Ok(m) => {
                                if let Some(data) = gate.admit(m, relay, room) {
                                    batch.push(data);
                                }
                            }
                            Err(TryRecvError::Lagged(n)) => {
                                metrics::add(&relay.metrics.lagged_messages, n);
                                *gate.waiting_keyframe = true;
                                room.request_keyframe(Instant::now());
                            }
                            Err(TryRecvError::Closed) => {
                                closed_now = true;
                                break;
                            }
                            Err(TryRecvError::Empty) => break,
                        }
                    }
                    if closed_now && batch.is_empty() {
                        break;
                    }
                }
                if batch.is_empty() {
                    continue;
                }
                let count = batch.len() as u64;
                let len: u64 = batch.iter().map(|b| b.len() as u64).sum();
                let write = async {
                    for data in batch.drain(..) {
                        sink.feed(Message::Binary(data)).await?;
                    }
                    sink.flush().await
                };
                match tokio::time::timeout(send_timeout, write).await {
                    Ok(Ok(())) => {
                        metrics::add(&relay.metrics.messages_out, count);
                        metrics::add(&relay.metrics.bytes_out, len);
                    }
                    Ok(Err(_)) => break,
                    Err(_) => {
                        metrics::inc(&relay.metrics.slow_viewers_disconnected);
                        break;
                    }
                }
            }
            _ = closed.changed() => {
                send_error(&mut sink, ErrorCode::RoomClosed, "sala encerrada").await;
                break;
            }
            _ = revocations.changed() => {
                if relay.is_revoked(claims) {
                    send_error(&mut sink, ErrorCode::Unauthorized, "token revogado").await;
                    break;
                }
            }
            () = &mut idle => {
                send_error(&mut sink, ErrorCode::IdleTimeout, "inatividade").await;
                break;
            }
            Ok(()) = presence.changed() => {
                // Só a versão mais recente importa: mudanças em rajada viram um envio.
                let msg = presence.borrow_and_update().clone();
                let len = msg.len() as u64;
                match tokio::time::timeout(send_timeout, sink.send(Message::Binary(msg))).await {
                    Ok(Ok(())) => {
                        metrics::inc(&relay.metrics.messages_out);
                        metrics::add(&relay.metrics.bytes_out, len);
                    }
                    _ => break,
                }
            }
        }
    }
    tracing::debug!(room = %room.id, session = %session_id, "espectador desconectado");
}
