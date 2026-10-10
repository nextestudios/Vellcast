//! Estado de mídia de uma sala, sempre em memória.
//!
//! Cada mensagem do transmissor é distribuída uma única vez por um
//! `broadcast` limitado: todos os espectadores recebem o mesmo `Bytes`
//! (sem cópia nem recodificação). Publicar e inscrever um espectador
//! acontecem sob o mesmo lock, para que o snapshot inicial (configuração +
//! GOP atual) e o fluxo ao vivo não tenham lacunas nem duplicatas.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use vellcast_protocol::messages::{
    ErrorCode, Participant, ParticipantRole, Presence, ShareInfo, encode_json,
};
use vellcast_protocol::{Header, MessageType, encode};

/// Máximo de participantes listados em uma mensagem `Presence` (o total
/// continua exato em `count`).
pub const MAX_PRESENCE_LISTED: usize = 100;

/// Nonces de tokens de compartilhamento já usados, guardados por sala até expirarem.
const MAX_HANDOFF_NONCES: usize = 256;

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

    /// `ShareChanged` com quem compartilha agora.
    pub fn share_changed(info: &ShareInfo) -> Self {
        let data = encode_json(MessageType::ShareChanged, 0, info).unwrap_or_default();
        Self {
            data,
            kind: MessageType::ShareChanged,
            keyframe: false,
            received_at: Instant::now(),
        }
    }
}

/// Por que um transmissor perdeu a vaga.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kick {
    /// Outra conexão com o mesmo token assumiu (reconexão).
    SameToken,
    /// Outro participante começou a compartilhar (troca explícita).
    Replaced { by: Option<String> },
}

/// Quem pede a vaga de transmissor, segundo o token.
#[derive(Debug, Clone, Default)]
pub struct PublisherIdentity {
    pub nonce: String,
    /// Nome exibido aos espectadores (opcional).
    pub name: Option<String>,
    /// Pode tomar a vaga de outro token ativo, só no primeiro uso.
    pub handoff: bool,
    /// Expiração do token (Unix, segundos).
    pub exp: u64,
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
    name: Option<String>,
    keyframe_tx: mpsc::Sender<()>,
    kick: Option<oneshot::Sender<Kick>>,
}

struct Inner {
    state: RoomState,
    publisher: Option<PublisherSlot>,
    /// Nonce do último token que transmitiu (troca de transmissor descarta a configuração).
    last_nonce: Option<String>,
    /// Tokens de compartilhamento já usados → expiração: uma reconexão nunca
    /// toma a vaga de quem assumiu depois.
    handoff_used: HashMap<String, u64>,
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
    /// Disparado quando outra conexão assume o lugar (mesmo token ou troca de
    /// participante). Erro: a vaga foi removida (sala encerrada).
    pub kicked: oneshot::Receiver<Kick>,
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
    /// Participantes ativos (S-PRESENCE), separado do estado de mídia.
    presence: Mutex<PresenceBook>,
    /// Última mensagem `Presence` serializada; espectadores enviam a mais recente.
    presence_tx: watch::Sender<Bytes>,
}

/// Uma sessão de participante: um token (nonce) em um papel. Várias conexões
/// com o mesmo token (reconexão antes de a anterior cair) contam como uma só.
struct PresenceEntry {
    id: String,
    name: Option<String>,
    role: ParticipantRole,
    connections: usize,
    /// Ordem de chegada, para uma lista estável.
    seq: u64,
}

#[derive(Default)]
struct PresenceBook {
    entries: HashMap<(ParticipantRole, String), PresenceEntry>,
    next_seq: u64,
}

impl PresenceBook {
    fn snapshot(&self) -> Presence {
        let mut list: Vec<&PresenceEntry> = self.entries.values().collect();
        list.sort_by_key(|e| (e.role, e.seq));
        Presence {
            count: list.len(),
            participants: list
                .into_iter()
                .take(MAX_PRESENCE_LISTED)
                .map(|e| Participant {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    role: e.role,
                })
                .collect(),
        }
    }
}

/// Mantém um participante na lista enquanto existir; ao ser descartado
/// (fim da conexão, por qualquer caminho), a presença é atualizada.
pub struct PresenceGuard {
    room: Arc<LiveRoom>,
    key: (ParticipantRole, String),
}

impl Drop for PresenceGuard {
    fn drop(&mut self) {
        self.room.leave_presence(&self.key);
    }
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
        let (presence_tx, _) = watch::channel(presence_message(&PresenceBook::default()));
        Self {
            presence: Mutex::new(PresenceBook::default()),
            presence_tx,
            id,
            max_viewers,
            inner: Mutex::new(Inner {
                state: RoomState::Idle,
                publisher: None,
                last_nonce: None,
                handoff_used: HashMap::new(),
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
        let identity = PublisherIdentity {
            nonce: token_nonce.to_owned(),
            ..PublisherIdentity::default()
        };
        self.attach_publisher_as(session_id, &identity, 0)
    }

    /// Registra o transmissor conforme o token. Continua havendo um só por sala:
    /// - o mesmo token (reconexão) substitui a própria conexão anterior;
    /// - um token de compartilhamento (`handoff`) no primeiro uso toma a vaga de
    ///   outro transmissor ativo, que recebe [`Kick::Replaced`] (troca explícita);
    /// - qualquer outro caso com a vaga ocupada é recusado (`PublisherExists`).
    ///
    /// Os espectadores recebem `ShareChanged` sempre que quem transmite muda.
    pub fn attach_publisher_as(
        &self,
        session_id: &str,
        identity: &PublisherIdentity,
        now_unix: u64,
    ) -> Result<PublisherLease, ErrorCode> {
        let mut inner = self.inner();
        if inner.state == RoomState::Closed {
            return Err(ErrorCode::RoomClosed);
        }
        let first_use = identity.handoff && !inner.handoff_used.contains_key(&identity.nonce);
        let replaces_other = inner
            .publisher
            .as_ref()
            .is_some_and(|p| p.token_nonce != identity.nonce);
        if replaces_other && !first_use {
            return Err(ErrorCode::PublisherExists);
        }
        if identity.handoff {
            remember_handoff(
                &mut inner.handoff_used,
                &identity.nonce,
                identity.exp,
                now_unix,
            );
        }
        if let Some(mut old) = inner.publisher.take()
            && let Some(kick) = old.kick.take()
        {
            let reason = if replaces_other {
                Kick::Replaced {
                    by: identity.name.clone(),
                }
            } else {
                Kick::SameToken
            };
            let _ = kick.send(reason);
        }
        let new_sharer = inner.last_nonce.as_deref() != Some(identity.nonce.as_str());
        if new_sharer {
            // Outro transmissor: a configuração do anterior não vale para o novo
            // fluxo (codec, resolução e linha do tempo podem mudar).
            inner.video_init = None;
            inner.audio_init = None;
            inner.last_nonce = Some(identity.nonce.clone());
        }
        let (keyframe_tx, keyframe_rx) = mpsc::channel(1);
        let (kick, kicked) = oneshot::channel();
        inner.publisher = Some(PublisherSlot {
            session_id: session_id.to_owned(),
            token_nonce: identity.nonce.clone(),
            name: identity.name.clone(),
            keyframe_tx,
            kick: Some(kick),
        });
        inner.state = RoomState::Live;
        inner.paused_since = None;
        inner.gop.clear();
        inner.last_activity = Instant::now();
        inner.last_keyframe_request = None;
        if new_sharer {
            let info = ShareInfo {
                name: identity.name.clone(),
                handoff: replaces_other,
            };
            let _ = self.tx.send(Relayed::share_changed(&info));
        }
        Ok(PublisherLease {
            keyframe_rx,
            kicked,
        })
    }

    /// Nome de quem compartilha agora (`None` sem transmissor ou sem nome).
    pub fn sharer_name(&self) -> Option<String> {
        self.inner().publisher.as_ref().and_then(|p| p.name.clone())
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
            inner.last_nonce = None;
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
        self.publish_locked(&mut inner, msg);
    }

    /// Como [`Self::publish`], mas só se `session_id` ainda for o transmissor
    /// da sala: quadros que uma sessão substituída ainda tinha em mãos não
    /// chegam aos espectadores do novo fluxo. Devolve `false` se descartou.
    pub fn publish_from(&self, session_id: &str, msg: Relayed) -> bool {
        let mut inner = self.inner();
        if !inner
            .publisher
            .as_ref()
            .is_some_and(|p| p.session_id == session_id)
        {
            return false;
        }
        self.publish_locked(&mut inner, msg);
        true
    }

    fn publish_locked(&self, inner: &mut Inner, msg: Relayed) {
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
        if let Some(p) = inner.publisher.as_ref().filter(|_| live) {
            // Quem compartilha, antes da configuração (a interface já mostra o nome).
            let info = ShareInfo {
                name: p.name.clone(),
                handoff: false,
            };
            snapshot.push(Relayed::share_changed(&info).data);
        }
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
            inner.last_nonce = None;
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

    fn presence_book(&self) -> std::sync::MutexGuard<'_, PresenceBook> {
        self.presence.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Registra uma conexão de participante. `token_nonce` identifica a sessão:
    /// reconexões com o mesmo token não duplicam a entrada. Devolve um guarda
    /// que remove a conexão ao ser descartado.
    pub fn join_presence(
        self: &Arc<Self>,
        token_nonce: &str,
        role: ParticipantRole,
        name: Option<String>,
    ) -> PresenceGuard {
        let key = (role, token_nonce.to_owned());
        let mut book = self.presence_book();
        let seq = book.next_seq;
        let mut inserted = false;
        let entry = book.entries.entry(key.clone()).or_insert_with(|| {
            inserted = true;
            PresenceEntry {
                id: crate::random_id(8),
                name: None,
                role,
                connections: 0,
                seq,
            }
        });
        entry.connections += 1;
        let renamed = name.is_some() && entry.name != name;
        if renamed {
            entry.name = name;
        }
        if inserted {
            book.next_seq += 1;
        }
        if inserted || renamed {
            self.presence_tx.send_replace(presence_message(&book));
        }
        PresenceGuard {
            room: Arc::clone(self),
            key,
        }
    }

    fn leave_presence(&self, key: &(ParticipantRole, String)) {
        let mut book = self.presence_book();
        let Some(entry) = book.entries.get_mut(key) else {
            return;
        };
        entry.connections = entry.connections.saturating_sub(1);
        if entry.connections == 0 {
            book.entries.remove(key);
            self.presence_tx.send_replace(presence_message(&book));
        }
    }

    /// Lista atual de participantes.
    pub fn presence(&self) -> Presence {
        self.presence_book().snapshot()
    }

    /// Mensagem `Presence` mais recente; muda a cada entrada ou saída.
    pub fn presence_signal(&self) -> watch::Receiver<Bytes> {
        self.presence_tx.subscribe()
    }
}

fn presence_message(book: &PresenceBook) -> Bytes {
    encode_json(MessageType::Presence, 0, &book.snapshot()).unwrap_or_default()
}

/// Registra o uso de um token de compartilhamento, limpando os expirados e
/// respeitando o limite (sai o que expira primeiro).
fn remember_handoff(used: &mut HashMap<String, u64>, nonce: &str, exp: u64, now_unix: u64) {
    if used.contains_key(nonce) {
        return;
    }
    used.retain(|_, e| *e > now_unix);
    if used.len() >= MAX_HANDOFF_NONCES
        && let Some(oldest) = used.iter().min_by_key(|(_, e)| **e).map(|(n, _)| n.clone())
    {
        used.remove(&oldest);
    }
    used.insert(nonce.to_owned(), exp);
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
        let kinds: Vec<MessageType> = join
            .snapshot
            .iter()
            .map(|b| {
                Header::read(b)
                    .map(|h| h.kind)
                    .unwrap_or(MessageType::Error)
            })
            .collect();
        assert_eq!(
            kinds[0],
            MessageType::ShareChanged,
            "quem compartilha vem primeiro"
        );
        let lens: Vec<usize> = join.snapshot[1..].iter().map(Bytes::len).collect();
        assert_eq!(lens, vec![1, 2, 4, 6]);
        assert!(join.has_keyframe);
    }

    fn sharer(nonce: &str, name: &str) -> PublisherIdentity {
        PublisherIdentity {
            nonce: nonce.into(),
            name: Some(name.into()),
            handoff: true,
            exp: 1_000,
        }
    }

    fn next_kind(rx: &mut broadcast::Receiver<Relayed>) -> Option<MessageType> {
        rx.try_recv().ok().map(|m| m.kind)
    }

    #[test]
    fn share_token_hands_off_once_and_never_on_reconnect() {
        let r = room();
        let mut viewer = r.join_viewer().rx;
        let mut ana = r
            .attach_publisher_as("s1", &sharer("ana", "Ana"), 10)
            .expect("ana");
        assert_eq!(next_kind(&mut viewer), Some(MessageType::ShareChanged));
        r.publish(frame(MessageType::VideoInit, false, 10));
        r.publish(frame(MessageType::VideoFrame, true, 10));
        assert_eq!(r.sharer_name().as_deref(), Some("Ana"));

        // Bia assume: Ana é avisada, config e GOP de Ana somem, espectadores são avisados.
        let _bia = r
            .attach_publisher_as("s2", &sharer("bia", "Bia"), 10)
            .expect("troca");
        assert_eq!(
            ana.kicked.try_recv().ok(),
            Some(Kick::Replaced {
                by: Some("Bia".into())
            })
        );
        assert_eq!(r.sharer_name().as_deref(), Some("Bia"));
        assert_eq!(r.gop_bytes(), 0);
        let snapshot = r.join_viewer().snapshot;
        assert_eq!(snapshot.len(), 1, "sem configuração velha: só ShareChanged");
        while let Some(kind) = next_kind(&mut viewer) {
            if kind == MessageType::ShareChanged {
                break;
            }
        }
        // Quadros que a sessão antiga ainda tinha em mãos são descartados.
        assert!(!r.publish_from("s1", frame(MessageType::VideoFrame, true, 5)));
        assert!(r.publish_from("s2", frame(MessageType::VideoInit, false, 5)));
        // A saída tardia da sessão de Ana não derruba a de Bia.
        r.detach_publisher("s1", false);
        assert_eq!(r.state(), RoomState::Live);

        // Reconexão do token de Ana (já usado) não toma a vaga de volta.
        assert_eq!(
            r.attach_publisher_as("s3", &sharer("ana", "Ana"), 10).err(),
            Some(ErrorCode::PublisherExists)
        );
        // Token comum (sem troca) também não toma a vaga.
        assert_eq!(
            r.attach_publisher("s4", "outro").err(),
            Some(ErrorCode::PublisherExists)
        );
    }

    #[test]
    fn stopping_or_dropping_releases_the_slot() {
        let r = room();
        let _a = r
            .attach_publisher_as("s1", &sharer("a", "A"), 10)
            .expect("a");
        r.detach_publisher("s1", true);
        assert_eq!(r.state(), RoomState::Idle);
        assert_eq!(r.sharer_name(), None);
        // Vaga livre: até um token sem troca pode transmitir.
        let _b = r.attach_publisher("s2", "b").expect("b");
        r.detach_publisher("s2", false);
        assert_eq!(r.state(), RoomState::Paused);
        // Queda também libera a vaga para outro participante, sem esperar a tolerância.
        let mut viewer = r.join_viewer().rx;
        let _c = r
            .attach_publisher_as("s3", &sharer("c", "C"), 10)
            .expect("c");
        assert_eq!(next_kind(&mut viewer), Some(MessageType::ShareChanged));
        assert_eq!(r.state(), RoomState::Live);
    }

    #[test]
    fn handoff_registry_is_bounded_and_prunes_expired() {
        let mut used = HashMap::new();
        remember_handoff(&mut used, "velho", 5, 0);
        remember_handoff(&mut used, "novo", 50, 10);
        assert!(!used.contains_key("velho"), "expirado sai");
        for i in 0..(MAX_HANDOFF_NONCES + 10) {
            remember_handoff(&mut used, &format!("n{i}"), 100 + i as u64, 10);
        }
        assert_eq!(used.len(), MAX_HANDOFF_NONCES);
        assert!(!used.contains_key("novo"), "o que expira primeiro sai");
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

    fn decode_presence(msg: &Bytes) -> Presence {
        let (h, p) =
            vellcast_protocol::decode(msg, vellcast_protocol::MAX_MESSAGE_LEN).expect("decode");
        assert_eq!(h.kind, MessageType::Presence);
        serde_json::from_slice(&p).expect("json")
    }

    #[test]
    fn presence_join_leave_and_reconnect_without_duplicates() {
        let r = Arc::new(room());
        let mut signal = r.presence_signal();
        assert_eq!(decode_presence(&signal.borrow_and_update()).count, 0);

        let host = r.join_presence("pub", ParticipantRole::Publisher, Some("Host".into()));
        let ana = r.join_presence("tok-ana", ParticipantRole::Viewer, Some("Ana".into()));
        assert!(signal.has_changed().unwrap_or(false));
        let p = decode_presence(&signal.borrow_and_update());
        assert_eq!(p.count, 2);
        assert_eq!(p.participants[0].role, ParticipantRole::Publisher);
        assert_eq!(p.participants[0].name.as_deref(), Some("Host"));
        assert_eq!(p.participants[1].name.as_deref(), Some("Ana"));
        let ana_id = p.participants[1].id.clone();

        // Reconexão com o mesmo token antes de a conexão antiga cair: uma entrada só.
        let ana_again = r.join_presence("tok-ana", ParticipantRole::Viewer, Some("Ana".into()));
        assert!(!signal.has_changed().unwrap_or(true), "nada mudou");
        drop(ana);
        assert!(!signal.has_changed().unwrap_or(true));
        let p = r.presence();
        assert_eq!(p.count, 2);
        assert_eq!(p.participants[1].id, ana_id, "id estável na reconexão");

        drop(ana_again);
        let p = decode_presence(&signal.borrow_and_update());
        assert_eq!(p.count, 1);
        assert_eq!(p.participants[0].role, ParticipantRole::Publisher);

        // Mesmo token em papéis diferentes (dono transmitindo e assistindo) são duas entradas.
        let owner_view = r.join_presence("pub", ParticipantRole::Viewer, None);
        assert_eq!(r.presence().count, 2);
        drop(owner_view);
        drop(host);
        assert_eq!(decode_presence(&signal.borrow_and_update()).count, 0);
    }

    #[test]
    fn presence_list_is_bounded_but_count_is_exact() {
        let r = Arc::new(LiveRoom::new(
            "r".into(),
            500,
            16,
            1000,
            Duration::from_millis(500),
        ));
        let guards: Vec<PresenceGuard> = (0..MAX_PRESENCE_LISTED + 5)
            .map(|i| r.join_presence(&format!("t{i}"), ParticipantRole::Viewer, None))
            .collect();
        let p = r.presence();
        assert_eq!(p.count, MAX_PRESENCE_LISTED + 5);
        assert_eq!(p.participants.len(), MAX_PRESENCE_LISTED);
        drop(guards);
        assert_eq!(r.presence().count, 0);
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
