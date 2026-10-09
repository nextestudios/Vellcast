//! Estado de mídia de uma sala, sempre em memória.
//!
//! Cada mensagem do transmissor é distribuída uma única vez por um
//! `broadcast` limitado: todos os espectadores recebem o mesmo `Bytes`
//! (sem cópia nem recodificação). Publicar e inscrever um espectador
//! acontecem sob o mesmo lock, para que o snapshot inicial (configuração +
//! GOP atual) e o fluxo ao vivo não tenham lacunas nem duplicatas.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use vellcast_protocol::messages::ErrorCode;
use vellcast_protocol::{Header, MessageType, encode};

/// Uma mensagem do transmissor pronta para distribuição.
#[derive(Debug, Clone)]
pub struct Relayed {
    pub data: Bytes,
    pub kind: MessageType,
    pub keyframe: bool,
    /// Instante de chegada ao relay (para descartar quadros velhos na fila).
    pub received_at: Instant,
}

impl Relayed {
    pub fn control(kind: MessageType, room_stream: u32) -> Self {
        let data = encode(&Header::new(kind, room_stream), &[]).unwrap_or_default();
        Self {
            data,
            kind,
            keyframe: false,
            received_at: Instant::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomState {
    /// Sem transmissão.
    Idle,
    Live,
    /// Transmissor caiu; aguardando reconexão dentro do período de tolerância.
    Paused,
    Closed,
}

/// Cache do GOP atual (do último keyframe em diante), limitado em bytes.
#[derive(Debug, Default)]
pub struct GopCache {
    frames: Vec<Bytes>,
    bytes: usize,
    max_bytes: usize,
    valid: bool,
}

impl GopCache {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ..Self::default()
        }
    }

    pub fn push(&mut self, keyframe: bool, data: &Bytes) {
        if keyframe {
            self.clear();
            self.valid = true;
        }
        if !self.valid {
            return;
        }
        if self.bytes + data.len() > self.max_bytes {
            // GOP grande demais: novos espectadores esperam o próximo keyframe.
            self.clear();
            return;
        }
        self.bytes += data.len();
        self.frames.push(data.clone());
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.bytes = 0;
        self.valid = false;
    }

    pub fn frames(&self) -> &[Bytes] {
        &self.frames
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

struct PublisherSlot {
    session_id: String,
    token_nonce: String,
    keyframe_tx: mpsc::Sender<()>,
    kick: Option<oneshot::Sender<()>>,
}

struct Inner {
    state: RoomState,
    publisher: Option<PublisherSlot>,
    paused_since: Option<Instant>,
    video_init: Option<Bytes>,
    audio_init: Option<Bytes>,
    gop: GopCache,
    last_activity: Instant,
    last_keyframe_request: Option<Instant>,
}

/// O que um transmissor recém-conectado recebe.
pub struct PublisherLease {
    pub keyframe_rx: mpsc::Receiver<()>,
    /// Disparado quando outra conexão com o mesmo token assume o lugar.
    pub kicked: oneshot::Receiver<()>,
}

/// O que um espectador recém-conectado recebe.
pub struct ViewerJoin {
    /// Configuração e GOP atual, a enviar antes do fluxo ao vivo.
    pub snapshot: Vec<Bytes>,
    pub has_keyframe: bool,
    pub rx: broadcast::Receiver<Relayed>,
    pub state: RoomState,
}

pub struct LiveRoom {
    pub id: String,
    pub max_viewers: usize,
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Relayed>,
    closed: watch::Sender<bool>,
    /// Incrementado a cada revogação: conexões ativas reavaliam seus tokens.
    revocations: watch::Sender<u64>,
    viewers: AtomicUsize,
    keyframe_interval: Duration,
}

impl LiveRoom {
    pub fn new(
        id: String,
        max_viewers: usize,
        queue: usize,
        gop_bytes: usize,
        keyframe_interval: Duration,
    ) -> Self {
        let (tx, _) = broadcast::channel(queue.max(1));
        let (closed, _) = watch::channel(false);
        let (revocations, _) = watch::channel(0);
        Self {
            id,
            max_viewers,
            inner: Mutex::new(Inner {
                state: RoomState::Idle,
                publisher: None,
                paused_since: None,
                video_init: None,
                audio_init: None,
                gop: GopCache::new(gop_bytes),
                last_activity: Instant::now(),
                last_keyframe_request: None,
            }),
            tx,
            closed,
            revocations,
            viewers: AtomicUsize::new(0),
            keyframe_interval,
        }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn state(&self) -> RoomState {
        self.inner().state
    }

    pub fn viewer_count(&self) -> usize {
        self.viewers.load(Ordering::Relaxed)
    }

    pub fn closed_signal(&self) -> watch::Receiver<bool> {
        self.closed.subscribe()
    }

    /// Reserva uma vaga de espectador. Devolve `false` se a sala está cheia.
    pub fn reserve_viewer(&self) -> bool {
        let prev = self.viewers.fetch_add(1, Ordering::AcqRel);
        if prev >= self.max_viewers {
            self.viewers.fetch_sub(1, Ordering::AcqRel);
            return false;
        }
        true
    }

    pub fn release_viewer(&self) {
        self.viewers.fetch_sub(1, Ordering::AcqRel);
        self.inner().last_activity = Instant::now();
    }

    /// Registra o transmissor. Só um por sala; uma reconexão com o mesmo
    /// token (mesmo nonce) substitui a conexão anterior.
    pub fn attach_publisher(
        &self,
        session_id: &str,
        token_nonce: &str,
    ) -> Result<PublisherLease, ErrorCode> {
        let mut inner = self.inner();
        match inner.state {
            RoomState::Closed => return Err(ErrorCode::RoomClosed),
            _ if inner
                .publisher
                .as_ref()
                .is_some_and(|p| p.token_nonce != token_nonce) =>
            {
                return Err(ErrorCode::PublisherExists);
            }
            _ => {}
        }
        if let Some(mut old) = inner.publisher.take()
            && let Some(kick) = old.kick.take()
        {
            let _ = kick.send(());
        }
        let (keyframe_tx, keyframe_rx) = mpsc::channel(1);
        let (kick, kicked) = oneshot::channel();
        inner.publisher = Some(PublisherSlot {
            session_id: session_id.to_owned(),
            token_nonce: token_nonce.to_owned(),
            keyframe_tx,
            kick: Some(kick),
        });
        inner.state = RoomState::Live;
        inner.paused_since = None;
        inner.gop.clear();
        inner.last_activity = Instant::now();
        Ok(PublisherLease {
            keyframe_rx,
            kicked,
        })
    }

    /// Libera o transmissor. `graceful` = enviou `StreamEnd`; caso contrário,
    /// a sala entra em pausa e aguarda a reconexão.
    pub fn detach_publisher(&self, session_id: &str, graceful: bool) {
        let mut inner = self.inner();
        if !inner
            .publisher
            .as_ref()
            .is_some_and(|p| p.session_id == session_id)
        {
            return; // Já substituído por outra conexão.
        }
        inner.publisher = None;
        inner.gop.clear();
        inner.last_activity = Instant::now();
        if inner.state == RoomState::Closed {
            return;
        }
        let msg = if graceful {
            inner.state = RoomState::Idle;
            inner.video_init = None;
            inner.audio_init = None;
            MessageType::StreamEnd
        } else {
            inner.state = RoomState::Paused;
            inner.paused_since = Some(Instant::now());
            MessageType::StreamPaused
        };
        let _ = self.tx.send(Relayed::control(msg, 0));
    }

    /// Distribui uma mensagem do transmissor e atualiza o cache.
    pub fn publish(&self, msg: Relayed) {
        let mut inner = self.inner();
        match msg.kind {
            MessageType::VideoInit => {
                inner.video_init = Some(msg.data.clone());
                inner.gop.clear();
            }
            MessageType::AudioInit => inner.audio_init = Some(msg.data.clone()),
            MessageType::VideoFrame => inner.gop.push(msg.keyframe, &msg.data),
            _ => {}
        }
        inner.last_activity = msg.received_at;
        // Erro só ocorre sem espectadores.
        let _ = self.tx.send(msg);
    }

    pub fn join_viewer(&self) -> ViewerJoin {
        let inner = self.inner();
        let rx = self.tx.subscribe();
        let mut snapshot = Vec::new();
        let live = inner.state == RoomState::Live;
        if live {
            snapshot.extend(inner.video_init.iter().cloned());
            snapshot.extend(inner.audio_init.iter().cloned());
        }
        let has_keyframe = live && inner.video_init.is_some() && !inner.gop.frames().is_empty();
        if has_keyframe {
            snapshot.extend(inner.gop.frames().iter().cloned());
        }
        ViewerJoin {
            snapshot,
            has_keyframe,
            rx,
            state: inner.state,
        }
    }

    /// Pede um keyframe ao transmissor, no máximo um por intervalo.
    pub fn request_keyframe(&self, now: Instant) -> bool {
        let mut inner = self.inner();
        if inner
            .last_keyframe_request
            .is_some_and(|t| now.saturating_duration_since(t) < self.keyframe_interval)
        {
            return false;
        }
        let Some(p) = &inner.publisher else {
            return false;
        };
        let sent = p.keyframe_tx.try_send(()).is_ok();
        if sent {
            inner.last_keyframe_request = Some(now);
        }
        sent
    }

    /// Manutenção periódica: encerra a pausa após o período de tolerância.
    /// Devolve `true` se a sala está ociosa há mais que `idle_ttl`.
    pub fn tick(&self, now: Instant, grace: Duration, idle_ttl: Duration) -> bool {
        let mut inner = self.inner();
        if inner.state == RoomState::Paused
            && inner
                .paused_since
                .is_some_and(|t| now.saturating_duration_since(t) >= grace)
        {
            inner.state = RoomState::Idle;
            inner.paused_since = None;
            inner.video_init = None;
            inner.audio_init = None;
            let _ = self.tx.send(Relayed::control(MessageType::StreamEnd, 0));
        }
        inner.state == RoomState::Idle
            && inner.publisher.is_none()
            && self.viewers.load(Ordering::Acquire) == 0
            && now.saturating_duration_since(inner.last_activity) >= idle_ttl
    }

    /// Avisa as conexões ativas que algum token da sala foi revogado.
    pub fn notify_revocation(&self) {
        self.revocations.send_modify(|n| *n = n.wrapping_add(1));
    }

    pub fn revocation_signal(&self) -> watch::Receiver<u64> {
        self.revocations.subscribe()
    }

    pub fn close(&self) {
        let mut inner = self.inner();
        if inner.state == RoomState::Closed {
            return;
        }
        inner.state = RoomState::Closed;
        inner.publisher = None;
        inner.gop.clear();
        let _ = self.tx.send(Relayed::control(MessageType::StreamEnd, 0));
        let _ = self.closed.send(true);
    }

    pub fn is_live(&self) -> bool {
        self.state() == RoomState::Live
    }

    pub fn gop_bytes(&self) -> usize {
        self.inner().gop.bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(kind: MessageType, keyframe: bool, len: usize) -> Relayed {
        Relayed {
            data: Bytes::from(vec![0u8; len]),
            kind,
            keyframe,
            received_at: Instant::now(),
        }
    }

    fn room() -> LiveRoom {
        LiveRoom::new("r".into(), 2, 16, 1000, Duration::from_millis(500))
    }

    #[test]
    fn gop_cache_is_bounded_and_restarts_at_keyframe() {
        let mut g = GopCache::new(100);
        g.push(false, &Bytes::from_static(&[0; 10]));
        assert!(g.frames().is_empty(), "delta sem keyframe não é cacheado");
        g.push(true, &Bytes::from_static(&[0; 40]));
        g.push(false, &Bytes::from_static(&[0; 40]));
        assert_eq!(g.bytes(), 80);
        g.push(false, &Bytes::from_static(&[0; 40]));
        assert_eq!(g.bytes(), 0, "excedeu o limite: cache descartado");
        g.push(false, &Bytes::from_static(&[0; 10]));
        assert!(g.frames().is_empty(), "espera o próximo keyframe");
        g.push(true, &Bytes::from_static(&[0; 10]));
        assert_eq!(g.frames().len(), 1);
    }

    #[test]
    fn single_publisher_and_takeover_with_same_token() {
        let r = room();
        let mut first = r.attach_publisher("s1", "nonce-a").expect("attach");
        assert_eq!(
            r.attach_publisher("s2", "nonce-b").err(),
            Some(ErrorCode::PublisherExists)
        );
        let _second = r.attach_publisher("s3", "nonce-a").expect("takeover");
        assert!(
            first.kicked.try_recv().is_ok(),
            "conexão antiga é derrubada"
        );
        // A saída tardia da sessão antiga não afeta a nova.
        r.detach_publisher("s1", false);
        assert_eq!(r.state(), RoomState::Live);
    }

    #[test]
    fn lifecycle_pause_grace_and_close() {
        let r = room();
        let _lease = r.attach_publisher("s1", "n").expect("attach");
        r.publish(frame(MessageType::VideoInit, false, 10));
        r.publish(frame(MessageType::VideoFrame, true, 10));
        r.detach_publisher("s1", false);
        assert_eq!(r.state(), RoomState::Paused);
        let t = Instant::now();
        assert!(!r.tick(t, Duration::from_secs(60), Duration::ZERO));
        assert_eq!(r.state(), RoomState::Paused);
        r.tick(
            t + Duration::from_secs(61),
            Duration::from_secs(60),
            Duration::from_secs(3600),
        );
        assert_eq!(r.state(), RoomState::Idle);
        r.close();
        assert_eq!(r.state(), RoomState::Closed);
        assert_eq!(
            r.attach_publisher("s2", "n").err(),
            Some(ErrorCode::RoomClosed)
        );
    }

    #[test]
    fn viewer_snapshot_starts_with_config_and_keyframe() {
        let r = room();
        let _lease = r.attach_publisher("s1", "n").expect("attach");
        r.publish(frame(MessageType::VideoInit, false, 1));
        r.publish(frame(MessageType::AudioInit, false, 2));
        r.publish(frame(MessageType::VideoFrame, false, 3)); // antes do keyframe: ignorado
        r.publish(frame(MessageType::VideoFrame, true, 4));
        r.publish(frame(MessageType::AudioFrame, false, 5)); // áudio não entra no GOP
        r.publish(frame(MessageType::VideoFrame, false, 6));
        let join = r.join_viewer();
        let lens: Vec<usize> = join.snapshot.iter().map(Bytes::len).collect();
        assert_eq!(lens, vec![1, 2, 4, 6]);
        assert!(join.has_keyframe);
    }

    #[test]
    fn viewer_limit() {
        let r = room();
        assert!(r.reserve_viewer());
        assert!(r.reserve_viewer());
        assert!(!r.reserve_viewer());
        r.release_viewer();
        assert!(r.reserve_viewer());
    }

    #[test]
    fn keyframe_requests_are_rate_limited() {
        let r = room();
        let mut lease = r.attach_publisher("s1", "n").expect("attach");
        let t = Instant::now();
        assert!(r.request_keyframe(t));
        assert!(!r.request_keyframe(t + Duration::from_millis(100)));
        assert!(lease.keyframe_rx.try_recv().is_ok());
        assert!(r.request_keyframe(t + Duration::from_millis(600)));
    }
}
