//! Servidor de validação da ETAPA 0.
//!
//! Reproduz mídia H.264/Opus pré-codificada em tempo real e a distribui
//! por WebSocket binário para a Discord Activity, sem recodificação.
//! Não é o relay de produção (ver `services/relay`).

mod stream;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;
use tower_http::services::{ServeDir, ServeFile};
use vellcast_protocol::{Header, MAX_CLIENT_MESSAGE_LEN, MessageType, decode, encode};

use crate::stream::{StreamInfo, TestStream, now_us};

/// Limite de espectadores simultâneos por transmissão de teste.
const MAX_VIEWERS_PER_STREAM: usize = 50;
/// Encerra a conexão se o cliente ficar este tempo sem enviar nada (ele envia Ping a cada 1 s).
const CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
/// Quadros de vídeo mais antigos que isso são descartados e o espectador
/// volta a sincronizar no próximo keyframe.
const MAX_VIDEO_AGE_US: u64 = 400_000;

#[derive(Default)]
struct ServerStats {
    viewers: AtomicUsize,
    connections_total: AtomicU64,
    lagged_messages: AtomicU64,
    stale_frames_dropped: AtomicU64,
    keyframe_requests: AtomicU64,
}

struct AppState {
    streams: HashMap<String, Arc<TestStream>>,
    stats: ServerStats,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,vellcast_stage0_server=debug".into()),
        )
        .init();

    let bind: SocketAddr = std::env::var("VELLCAST_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()
        .context("VELLCAST_BIND inválido")?;
    let media_dir = PathBuf::from(
        std::env::var("VELLCAST_MEDIA_DIR").unwrap_or_else(|_| "tests/fixtures/media".into()),
    );
    let static_dir = PathBuf::from(
        std::env::var("VELLCAST_STATIC_DIR")
            .unwrap_or_else(|_| "apps/discord-activity/dist".into()),
    );

    let streams = stream::load_all(&media_dir).with_context(
        || "falha ao carregar a mídia de teste; execute scripts/generate-test-media.sh",
    )?;
    if streams.is_empty() {
        anyhow::bail!("nenhum manifest em {}", media_dir.display());
    }
    for s in &streams {
        tracing::info!(
            name = %s.info.name,
            codec = %s.info.codec,
            width = s.info.width,
            height = s.info.height,
            fps = s.info.fps,
            bitrate_kbps = s.info.video_bitrate_bps / 1000,
            audio = s.info.has_audio,
            "transmissão de teste carregada"
        );
        tokio::spawn(Arc::clone(s).run());
    }

    let state = Arc::new(AppState {
        streams: streams
            .into_iter()
            .map(|s| (s.info.name.clone(), s))
            .collect(),
        stats: ServerStats::default(),
    });

    let spa = ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html")));
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/api/streams", get(list_streams))
        .route("/api/metrics", get(metrics))
        .route("/ws", get(ws_handler))
        .fallback_service(spa)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, static_dir = %static_dir.display(), "servidor da ETAPA 0 ouvindo");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn list_streams(State(state): State<Arc<AppState>>) -> Json<Vec<StreamInfo>> {
    let mut list: Vec<_> = state.streams.values().map(|s| s.info.clone()).collect();
    list.sort_by_key(|s| s.id);
    Json(list)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamMetrics {
    name: String,
    viewers: usize,
    frames_sent: u64,
    bytes_broadcast: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metrics {
    viewers: usize,
    connections_total: u64,
    lagged_messages: u64,
    stale_frames_dropped: u64,
    keyframe_requests: u64,
    streams: Vec<StreamMetrics>,
}

async fn metrics(State(state): State<Arc<AppState>>) -> Json<Metrics> {
    let s = &state.stats;
    let mut streams: Vec<_> = state
        .streams
        .values()
        .map(|t| StreamMetrics {
            name: t.info.name.clone(),
            viewers: t.sender.receiver_count(),
            frames_sent: t.stats.frames_sent.load(Ordering::Relaxed),
            bytes_broadcast: t.stats.bytes_broadcast.load(Ordering::Relaxed),
        })
        .collect();
    streams.sort_by(|a, b| a.name.cmp(&b.name));
    Json(Metrics {
        viewers: s.viewers.load(Ordering::Relaxed),
        connections_total: s.connections_total.load(Ordering::Relaxed),
        lagged_messages: s.lagged_messages.load(Ordering::Relaxed),
        stale_frames_dropped: s.stale_frames_dropped.load(Ordering::Relaxed),
        keyframe_requests: s.keyframe_requests.load(Ordering::Relaxed),
        streams,
    })
}

#[derive(Deserialize)]
struct WsQuery {
    stream: String,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    State(state): State<Arc<AppState>>,
) -> Response {
    let Some(stream) = state.streams.get(&query.stream).cloned() else {
        return (StatusCode::NOT_FOUND, "transmissão não encontrada").into_response();
    };
    if stream.sender.receiver_count() >= MAX_VIEWERS_PER_STREAM {
        return (StatusCode::SERVICE_UNAVAILABLE, "limite de espectadores").into_response();
    }
    ws.max_message_size(MAX_CLIENT_MESSAGE_LEN)
        .max_frame_size(MAX_CLIENT_MESSAGE_LEN)
        .on_upgrade(move |socket| async move {
            state.stats.viewers.fetch_add(1, Ordering::Relaxed);
            state
                .stats
                .connections_total
                .fetch_add(1, Ordering::Relaxed);
            let result = serve_viewer(socket, &stream, &state.stats).await;
            state.stats.viewers.fetch_sub(1, Ordering::Relaxed);
            match result {
                Ok(reason) => {
                    tracing::debug!(stream = %stream.info.name, reason, "espectador saiu")
                }
                Err(err) => {
                    tracing::debug!(stream = %stream.info.name, %err, "conexão encerrada com erro")
                }
            }
        })
}

async fn serve_viewer(
    socket: WebSocket,
    stream: &TestStream,
    stats: &ServerStats,
) -> Result<&'static str> {
    // Inscreve antes de enviar a configuração para não perder quadros.
    let mut rx = stream.sender.subscribe();
    let (mut sink, mut source) = socket.split();
    for init in &stream.init_messages {
        sink.send(Message::Binary(init.clone())).await?;
    }

    let mut waiting_keyframe = true;
    let idle = tokio::time::sleep(CLIENT_IDLE_TIMEOUT);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            incoming = source.next() => {
                let Some(incoming) = incoming else { return Ok("cliente desconectou") };
                idle.as_mut().reset(tokio::time::Instant::now() + CLIENT_IDLE_TIMEOUT);
                match incoming? {
                    Message::Binary(data) => {
                        if let Some(reply) = handle_client_message(&data, stats) {
                            sink.send(Message::Binary(reply)).await?;
                        }
                    }
                    Message::Close(_) => return Ok("close recebido"),
                    _ => {}
                }
            }
            outgoing = rx.recv() => {
                let message = match outgoing {
                    Ok(m) => m,
                    Err(RecvError::Lagged(n)) => {
                        // Fila cheia: as mensagens mais antigas foram descartadas.
                        stats.lagged_messages.fetch_add(n, Ordering::Relaxed);
                        waiting_keyframe = true;
                        continue;
                    }
                    Err(RecvError::Closed) => {
                        let end = encode(&Header::new(MessageType::StreamEnd, stream.info.id), &[])?;
                        sink.send(Message::Binary(end)).await?;
                        return Ok("transmissão encerrada");
                    }
                };
                let Ok(header) = Header::read(&message) else { continue };
                if header.kind == MessageType::VideoFrame {
                    if now_us().saturating_sub(header.sent_at_us) > MAX_VIDEO_AGE_US {
                        stats.stale_frames_dropped.fetch_add(1, Ordering::Relaxed);
                        waiting_keyframe = true;
                        continue;
                    }
                    if waiting_keyframe {
                        if !header.is_keyframe() {
                            continue;
                        }
                        waiting_keyframe = false;
                    }
                }
                sink.send(Message::Binary(message)).await?;
            }
            () = &mut idle => {
                let _ = sink
                    .send(Message::Close(Some(CloseFrame { code: 4000, reason: "idle timeout".into() })))
                    .await;
                return Ok("timeout de inatividade");
            }
        }
    }
}

fn handle_client_message(data: &Bytes, stats: &ServerStats) -> Option<Bytes> {
    let (header, payload) = decode(data, MAX_CLIENT_MESSAGE_LEN).ok()?;
    match header.kind {
        MessageType::Ping => {
            let mut pong = Header::new(MessageType::Pong, header.stream_id);
            pong.sequence = header.sequence;
            pong.sent_at_us = now_us();
            encode(&pong, &payload).ok()
        }
        MessageType::KeyframeRequest => {
            // A mídia de teste tem keyframes a cada 1 s; o pedido é apenas contabilizado.
            stats.keyframe_requests.fetch_add(1, Ordering::Relaxed);
            None
        }
        _ => None,
    }
}
