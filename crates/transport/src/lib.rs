//! Cliente de transporte do transmissor: envia uma transmissão ao relay.
//!
//! - Conexão WebSocket (TLS via rustls quando a URL é `wss://`).
//! - Autenticação pela primeira mensagem (`Auth`); o token nunca vai na URL.
//! - Fila de envio limitada ([`queue::SendQueue`]): sob pressão, descarta
//!   vídeo até o próximo keyframe e avisa o encoder.
//! - Reconexão com backoff exponencial e jitter; após reconectar, reenvia a
//!   configuração com a flag `DISCONTINUITY` e pede um keyframe ao encoder.
//!
//! As chamadas de envio não bloqueiam e podem vir da thread do encoder.

pub mod queue;

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use vellcast_protocol::messages::{
    AudioInit, AuthOk, AuthRequest, ConnectRole, ErrorBody, ErrorCode, VideoInit,
};
use vellcast_protocol::{Header, MAX_MESSAGE_LEN, MessageType, decode, encode, flags};

use crate::queue::{PushOutcome, SendQueue};

#[derive(Debug, Clone)]
pub struct PublisherConfig {
    /// URL do relay, ex.: `wss://relay.exemplo.com/ws`.
    pub url: String,
    pub token: String,
    pub queue_max_items: usize,
    pub queue_max_bytes: usize,
    pub connect_timeout: Duration,
    pub ping_interval: Duration,
    /// Atrasos entre tentativas; o último se repete. Cada um recebe ±20% de jitter.
    pub backoff: Vec<Duration>,
}

impl PublisherConfig {
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            token: token.into(),
            // ~2 s de 1080p60 + áudio.
            queue_max_items: 240,
            queue_max_bytes: 8 * 1024 * 1024,
            connect_timeout: Duration::from_secs(10),
            ping_interval: Duration::from_secs(2),
            backoff: [500, 1000, 2000, 4000, 8000]
                .into_iter()
                .map(Duration::from_millis)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherEvent {
    Connected {
        session_id: String,
    },
    Disconnected {
        reason: String,
    },
    /// O encoder deve gerar um keyframe (pedido do relay, reconexão ou descarte local).
    KeyframeRequested,
    /// Erro definitivo; o cliente parou de tentar.
    Fatal {
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Queued,
    Dropped,
}

#[derive(Debug, Default)]
struct Stats {
    messages_sent: AtomicU64,
    bytes_sent: AtomicU64,
    video_dropped: AtomicU64,
    audio_dropped: AtomicU64,
    reconnects: AtomicU64,
    last_rtt_us: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatsSnapshot {
    pub messages_sent: u64,
    pub bytes_sent: u64,
    pub video_dropped: u64,
    pub audio_dropped: u64,
    pub reconnects: u64,
    pub last_rtt_us: u64,
    pub queue_len: usize,
    pub queue_bytes: usize,
}

struct Shared {
    queue: Mutex<SendQueue>,
    notify: Notify,
    stats: Stats,
    sequence: AtomicU32,
    video_init: Mutex<Option<Bytes>>,
    audio_init: Mutex<Option<Bytes>>,
    finishing: AtomicBool,
    stop: Notify,
    events: mpsc::Sender<PublisherEvent>,
}

impl Shared {
    fn queue(&self) -> std::sync::MutexGuard<'_, SendQueue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: PublisherEvent) {
        // Eventos são descartados se a aplicação não os consumir (nunca bloqueia).
        let _ = self.events.try_send(event);
    }

    fn header(&self, kind: MessageType, pts_us: u64, flags: u16) -> Header {
        let mut h = Header::new(kind, 0);
        h.sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        h.pts_us = pts_us;
        h.sent_at_us = now_us();
        h.flags = flags;
        h
    }
}

pub struct Publisher {
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl Publisher {
    /// Inicia a conexão em segundo plano. Deve ser chamado dentro de um runtime Tokio.
    pub fn connect(config: PublisherConfig) -> (Self, mpsc::Receiver<PublisherEvent>) {
        let (events, rx) = mpsc::channel(64);
        let shared = Arc::new(Shared {
            queue: Mutex::new(SendQueue::new(
                config.queue_max_items,
                config.queue_max_bytes,
            )),
            notify: Notify::new(),
            stats: Stats::default(),
            sequence: AtomicU32::new(0),
            video_init: Mutex::new(None),
            audio_init: Mutex::new(None),
            finishing: AtomicBool::new(false),
            stop: Notify::new(),
            events,
        });
        let task = tokio::spawn(run(Arc::clone(&shared), config));
        (Self { shared, task }, rx)
    }

    fn set_init(&self, kind: MessageType, json: Vec<u8>) {
        let Ok(msg) = encode(&self.shared.header(kind, 0, 0), &json) else {
            return;
        };
        let slot = if kind == MessageType::VideoInit {
            &self.shared.video_init
        } else {
            &self.shared.audio_init
        };
        *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(msg.clone());
        self.shared.queue().push_control(msg);
        self.shared.notify.notify_one();
    }

    /// Define (ou altera) a configuração de vídeo. É reenviada a cada reconexão.
    pub fn set_video_init(&self, init: &VideoInit) {
        if let Ok(json) = serde_json::to_vec(init) {
            self.set_init(MessageType::VideoInit, json);
        }
    }

    pub fn set_audio_init(&self, init: &AudioInit) {
        if let Ok(json) = serde_json::to_vec(init) {
            self.set_init(MessageType::AudioInit, json);
        }
    }

    /// Enfileira uma unidade de acesso H.264 (Annex B).
    pub fn send_video(&self, access_unit: &[u8], pts_us: u64, keyframe: bool) -> SendOutcome {
        let f = if keyframe { flags::KEYFRAME } else { 0 };
        let Ok(msg) = encode(
            &self.shared.header(MessageType::VideoFrame, pts_us, f),
            access_unit,
        ) else {
            self.shared
                .stats
                .video_dropped
                .fetch_add(1, Ordering::Relaxed);
            return SendOutcome::Dropped;
        };
        let outcome = self.shared.queue().push_video(msg, keyframe);
        self.finish_push(outcome, &self.shared.stats.video_dropped)
    }

    /// Enfileira um pacote Opus.
    pub fn send_audio(&self, packet: &[u8], pts_us: u64) -> SendOutcome {
        let Ok(msg) = encode(
            &self.shared.header(MessageType::AudioFrame, pts_us, 0),
            packet,
        ) else {
            self.shared
                .stats
                .audio_dropped
                .fetch_add(1, Ordering::Relaxed);
            return SendOutcome::Dropped;
        };
        let outcome = self.shared.queue().push_audio(msg);
        self.finish_push(outcome, &self.shared.stats.audio_dropped)
    }

    fn finish_push(&self, outcome: PushOutcome, dropped: &AtomicU64) -> SendOutcome {
        match outcome {
            PushOutcome::Queued => {
                self.shared.notify.notify_one();
                SendOutcome::Queued
            }
            PushOutcome::Dropped { need_keyframe } => {
                dropped.fetch_add(1, Ordering::Relaxed);
                if need_keyframe {
                    self.shared.emit(PublisherEvent::KeyframeRequested);
                }
                SendOutcome::Dropped
            }
        }
    }

    pub fn stats(&self) -> StatsSnapshot {
        let s = &self.shared.stats;
        let l = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let q = self.shared.queue();
        StatsSnapshot {
            messages_sent: l(&s.messages_sent),
            bytes_sent: l(&s.bytes_sent),
            video_dropped: l(&s.video_dropped),
            audio_dropped: l(&s.audio_dropped),
            reconnects: l(&s.reconnects),
            last_rtt_us: l(&s.last_rtt_us),
            queue_len: q.len(),
            queue_bytes: q.bytes(),
        }
    }

    /// Envia `StreamEnd` (sala volta a ociosa imediatamente) e encerra.
    pub async fn finish(self, timeout: Duration) {
        self.shared.finishing.store(true, Ordering::Release);
        self.shared.stop.notify_one();
        if tokio::time::timeout(timeout, self.task).await.is_err() {
            tracing::warn!("encerramento do transmissor excedeu o tempo limite");
        }
    }

    /// Encerra sem `StreamEnd` (simula queda; a sala entra em pausa).
    pub fn abort(self) {
        self.task.abort();
    }
}

fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

fn jitter(d: Duration) -> Duration {
    let mut b = [0u8; 2];
    let r = if getrandom::fill(&mut b).is_ok() {
        f64::from(u16::from_le_bytes(b)) / f64::from(u16::MAX)
    } else {
        0.5
    };
    d.mul_f64(0.8 + 0.4 * r)
}

enum Outcome {
    /// `StreamEnd` enviado; não reconectar.
    Finished,
    Fatal(ErrorCode, String),
    /// `authed`: a sessão chegou a autenticar (zera o backoff).
    Retry {
        reason: String,
        authed: bool,
    },
}

async fn run(shared: Arc<Shared>, config: PublisherConfig) {
    // Garante o provedor criptográfico do rustls (ring) antes do primeiro TLS.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut attempt = 0usize;
    let mut reconnect = false;
    loop {
        match session(&shared, &config, reconnect).await {
            Outcome::Finished => return,
            Outcome::Fatal(code, message) => {
                tracing::warn!(?code, %message, "relay recusou a transmissão");
                shared.emit(PublisherEvent::Fatal { code, message });
                return;
            }
            Outcome::Retry { reason, authed } => {
                if shared.finishing.load(Ordering::Acquire) {
                    return;
                }
                if authed {
                    attempt = 0;
                }
                let base = config
                    .backoff
                    .get(attempt.min(config.backoff.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or(Duration::from_secs(1));
                let delay = jitter(base);
                tracing::info!(%reason, delay_ms = delay.as_millis() as u64, "reconectando ao relay");
                shared.emit(PublisherEvent::Disconnected { reason });
                attempt += 1;
                reconnect = true;
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    () = shared.stop.notified() => return,
                }
                shared.stats.reconnects.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn error_from(payload: &[u8]) -> (ErrorCode, String) {
    serde_json::from_slice::<ErrorBody>(payload)
        .map_or((ErrorCode::ProtocolError, "erro ilegível".into()), |e| {
            (e.code, e.message)
        })
}

async fn session(shared: &Shared, config: &PublisherConfig, reconnect: bool) -> Outcome {
    let retry = |reason: String, authed| Outcome::Retry { reason, authed };
    let ws = match tokio::time::timeout(
        config.connect_timeout,
        tokio_tungstenite::connect_async(config.url.as_str()),
    )
    .await
    {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return retry(format!("falha ao conectar: {e}"), false),
        Err(_) => return retry("tempo de conexão esgotado".into(), false),
    };
    let (mut sink, mut source) = ws.split();

    let auth = AuthRequest {
        token: config.token.clone(),
        role: ConnectRole::Publisher,
    };
    let Ok(auth) = serde_json::to_vec(&auth) else {
        return Outcome::Fatal(ErrorCode::Internal, "falha ao serializar Auth".into());
    };
    let Ok(auth) = encode(&Header::new(MessageType::Auth, 0), &auth) else {
        return Outcome::Fatal(ErrorCode::Internal, "token grande demais".into());
    };
    if let Err(e) = sink.send(Message::Binary(auth)).await {
        return retry(format!("falha ao autenticar: {e}"), false);
    }

    // Aguarda AuthOk ou Error.
    let first = tokio::time::timeout(config.connect_timeout, source.next()).await;
    let session_id = match first {
        Ok(Some(Ok(Message::Binary(data)))) => match decode(&data, MAX_MESSAGE_LEN) {
            Ok((h, p)) if h.kind == MessageType::AuthOk => serde_json::from_slice::<AuthOk>(&p)
                .map(|ok| ok.session_id)
                .unwrap_or_default(),
            Ok((h, p)) if h.kind == MessageType::Error => {
                let (code, message) = error_from(&p);
                return if code.is_fatal() {
                    Outcome::Fatal(code, message)
                } else {
                    retry(message, false)
                };
            }
            _ => return Outcome::Fatal(ErrorCode::ProtocolError, "resposta inesperada".into()),
        },
        _ => return retry("relay não respondeu à autenticação".into(), false),
    };
    shared.emit(PublisherEvent::Connected { session_id });

    // Reenvia a configuração e recomeça o vídeo a partir de um keyframe.
    shared.queue().reset_for_reconnect();
    let inits: Vec<Bytes> = [&shared.video_init, &shared.audio_init]
        .iter()
        .filter_map(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).clone())
        .collect();
    for init in inits {
        let mut data = init.to_vec();
        if reconnect {
            let f = u16::from_le_bytes([data[2], data[3]]) | flags::DISCONTINUITY;
            data[2..4].copy_from_slice(&f.to_le_bytes());
        }
        if let Err(e) = sink.send(Message::Binary(Bytes::from(data))).await {
            return retry(format!("falha ao enviar configuração: {e}"), true);
        }
    }
    shared.emit(PublisherEvent::KeyframeRequested);

    let mut ping = tokio::time::interval(config.ping_interval);
    ping.tick().await;
    loop {
        // Drena a fila em lote: `feed` enfileira no WebSocket e um único `flush`
        // escreve tudo no socket. `notify_one` guarda a permissão se ninguém
        // estiver esperando.
        let (mut count, mut bytes) = (0u64, 0u64);
        loop {
            // O lock da fila é liberado antes do `await` abaixo.
            let item = shared.queue().pop();
            let Some(item) = item else { break };
            bytes += item.data.len() as u64;
            count += 1;
            if let Err(e) = sink.feed(Message::Binary(item.data)).await {
                return retry(format!("falha no envio: {e}"), true);
            }
        }
        if count > 0 {
            if let Err(e) = sink.flush().await {
                return retry(format!("falha no envio: {e}"), true);
            }
            shared
                .stats
                .messages_sent
                .fetch_add(count, Ordering::Relaxed);
            shared.stats.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
        }
        if shared.finishing.load(Ordering::Acquire) {
            if let Ok(end) = encode(&shared.header(MessageType::StreamEnd, 0, 0), &[]) {
                let _ = sink.send(Message::Binary(end)).await;
            }
            let _ = sink.close().await;
            return Outcome::Finished;
        }
        tokio::select! {
            () = shared.notify.notified() => {}
            () = shared.stop.notified() => {}
            _ = ping.tick() => {
                let payload = now_us().to_le_bytes();
                if let Ok(p) = encode(&shared.header(MessageType::Ping, 0, 0), &payload)
                    && let Err(e) = sink.send(Message::Binary(p)).await
                {
                    return retry(format!("falha no ping: {e}"), true);
                }
            }
            incoming = source.next() => {
                let data = match incoming {
                    Some(Ok(Message::Binary(d))) => d,
                    Some(Ok(Message::Close(_))) | None => return retry("relay fechou a conexão".into(), true),
                    Some(Err(e)) => return retry(format!("erro de conexão: {e}"), true),
                    Some(Ok(_)) => continue,
                };
                let Ok((h, p)) = decode(&data, MAX_MESSAGE_LEN) else { continue };
                match h.kind {
                    MessageType::KeyframeRequest => shared.emit(PublisherEvent::KeyframeRequested),
                    MessageType::Pong if p.len() >= 8 => {
                        let mut t = [0u8; 8];
                        t.copy_from_slice(&p[..8]);
                        let rtt = now_us().saturating_sub(u64::from_le_bytes(t));
                        shared.stats.last_rtt_us.store(rtt, Ordering::Relaxed);
                    }
                    MessageType::Error => {
                        let (code, message) = error_from(&p);
                        return if code.is_fatal() {
                            Outcome::Fatal(code, message)
                        } else {
                            retry(message, true)
                        };
                    }
                    _ => {}
                }
            }
        }
    }
}
