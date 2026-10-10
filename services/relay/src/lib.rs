//! `vellcast-relay`: servidor de mídia do Vellcast.
//!
//! Recebe uma única transmissão codificada por sala e a distribui aos
//! espectadores autorizados, sem decodificar nem recodificar.
//! Arquitetura e limites: `docs/ARCHITECTURE.md` e `docs/STREAMING_PROTOCOL.md`.

pub mod api;
pub mod config;
pub mod discord;
pub mod entitlement;
pub mod metrics;
pub mod pairing;
pub mod ratelimit;
pub mod room;
pub mod store;
pub mod store_sqlite;
pub mod token;
pub mod web;
pub mod ws;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::http::HeaderMap;
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::config::Config;
use crate::metrics::Metrics;
use crate::ratelimit::IpRateLimiter;
use crate::room::LiveRoom;
use crate::store::{MemoryStore, RoomStore};
use crate::token::{Claims, Role, TokenSigner};

pub struct Relay {
    pub config: Config,
    pub signer: TokenSigner,
    pub store: Arc<dyn RoomStore>,
    rooms: RwLock<HashMap<String, Arc<LiveRoom>>>,
    pub metrics: Metrics,
    pub api_limiter: IpRateLimiter,
    pub ws_limiter: IpRateLimiter,
    /// Salas por conta no modo hospedado (limites de plano).
    pub account_rooms: entitlement::AccountRooms,
    started: Instant,
}

impl Relay {
    pub fn new(config: Config) -> Arc<Self> {
        Self::with_store(config, Arc::new(MemoryStore::default()))
    }

    /// Store conforme a configuração: SQLite em `db_path`, ou memória.
    pub fn from_config(config: Config) -> anyhow::Result<Arc<Self>> {
        let store: Arc<dyn RoomStore> = match &config.db_path {
            Some(path) => Arc::new(
                store_sqlite::SqliteStore::open(path)
                    .map_err(|e| anyhow::anyhow!("banco {}: {e}", path.display()))?,
            ),
            None => Arc::new(MemoryStore::default()),
        };
        Ok(Self::with_store(config, store))
    }

    pub fn with_store(config: Config, store: Arc<dyn RoomStore>) -> Arc<Self> {
        let relay = Arc::new(Self {
            signer: TokenSigner::with_previous(&config.secret, config.previous_secret.as_deref()),
            api_limiter: IpRateLimiter::new(config.api_burst, config.api_per_sec),
            ws_limiter: IpRateLimiter::new(config.ws_burst, config.ws_per_sec),
            config,
            store,
            rooms: RwLock::new(HashMap::new()),
            metrics: Metrics::default(),
            account_rooms: entitlement::AccountRooms::default(),
            started: Instant::now(),
        });
        relay.restore_rooms();
        relay
    }

    /// Recria o estado ao vivo das salas persistidas (após reinício do relay).
    fn restore_rooms(&self) {
        let now = unix_now();
        let mut restored = 0usize;
        for record in self.store.list() {
            if record.expires_at <= now {
                self.store.remove(&record.id);
                continue;
            }
            self.insert_room(Arc::new(self.new_live_room(&record.id, record.max_viewers)));
            restored += 1;
        }
        if restored > 0 {
            tracing::info!(restored, "salas restauradas do banco");
        }
    }

    pub(crate) fn new_live_room(&self, id: &str, max_viewers: usize) -> LiveRoom {
        let c = &self.config;
        LiveRoom::new(
            id.to_owned(),
            max_viewers,
            c.viewer_queue,
            c.gop_cache_bytes,
            c.keyframe_request_interval,
        )
    }

    /// `true` se o token foi revogado (por nonce ou pela revogação em massa de espectadores).
    ///
    /// Um token de compartilhamento derivado de um participante (`parent`) segue
    /// o participante: cai junto com o token de espectador de origem e com a
    /// revogação em massa de espectadores.
    pub fn is_revoked(&self, claims: &Claims) -> bool {
        self.store.get(&claims.room).is_some_and(|r| {
            let viewer_class = claims.role == Role::Viewer || claims.parent.is_some();
            r.is_revoked(&claims.nonce, viewer_class, claims.iat)
                || claims
                    .parent
                    .as_deref()
                    .is_some_and(|p| r.revoked.contains_key(p))
        })
    }

    pub fn room(&self, id: &str) -> Option<Arc<LiveRoom>> {
        self.rooms
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    fn insert_room(&self, room: Arc<LiveRoom>) {
        self.rooms
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(room.id.clone(), room);
    }

    /// Encerra a sala: espectadores e transmissor são desconectados.
    pub fn close_room(&self, id: &str, reason: &str) {
        let room = self
            .rooms
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.store.remove(id);
        if let Some(room) = room {
            room.close();
            metrics::inc(&self.metrics.rooms_closed);
            tracing::info!(room = %id, reason, "sala encerrada");
        }
    }

    fn live_rooms(&self) -> Vec<Arc<LiveRoom>> {
        self.rooms
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub fn metrics_snapshot(&self) -> metrics::Snapshot {
        let rooms = self.live_rooms();
        let live = rooms.iter().filter(|r| r.is_live()).count();
        let viewers = rooms.iter().map(|r| r.viewer_count()).sum();
        self.metrics
            .snapshot(self.started.elapsed().as_secs(), rooms.len(), live, viewers)
    }

    /// Manutenção: salas expiradas ou ociosas, pausas vencidas e rate limiters.
    pub fn sweep(&self) {
        let now = Instant::now();
        for id in self.store.sweep(unix_now()) {
            self.close_room(&id, "ttl expirado");
        }
        for room in self.live_rooms() {
            if room.tick(now, self.config.publisher_grace, self.config.room_idle_ttl) {
                self.close_room(&room.id, "ociosa");
            }
        }
        self.api_limiter.prune();
        self.ws_limiter.prune();
    }

    /// IP do cliente; com `trust_proxy`, usa o último salto de `X-Forwarded-For`.
    pub fn client_ip(&self, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
        if self.config.trust_proxy
            && let Some(ip) = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
                .and_then(|v| v.trim().parse().ok())
        {
            return ip;
        }
        peer.ip()
    }
}

pub fn router(relay: Arc<Relay>) -> Router {
    router_with_discord(relay, discord::DiscordConfig::from_env())
}

/// Router com a integração Discord explícita (testes usam uma API simulada).
pub fn router_with_discord(relay: Arc<Relay>, discord: Option<discord::DiscordConfig>) -> Router {
    let discord_routes = discord::router(Arc::clone(&relay), discord);
    let static_dir = relay.config.static_dir.clone();
    let api = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/metrics", get(api::metrics))
        .route("/ws", get(ws::upgrade))
        .route("/api/rooms", post(api::create_room))
        .route(
            "/api/rooms/{id}",
            get(api::get_room).delete(api::delete_room),
        )
        .route("/api/rooms/{id}/invites", post(api::create_invite))
        .route("/api/rooms/{id}/join", post(api::join_room))
        .route(
            "/api/rooms/{id}/publisher-token",
            post(api::publisher_token),
        )
        .route("/api/rooms/{id}/revoke", post(api::revoke))
        .route("/api/rooms/{id}/share", post(api::share))
        .with_state(relay)
        .merge(discord_routes);
    match static_dir {
        Some(dir) => web::with_player(api, &dir),
        None => api,
    }
}

/// Tarefa periódica de limpeza.
pub fn spawn_sweeper(relay: Arc<Relay>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(relay.config.cleanup_interval);
        loop {
            interval.tick().await;
            relay.sweep();
        }
    })
}

/// Serve o relay em um listener já aberto (usado pelo binário e pelos testes).
pub async fn serve(relay: Arc<Relay>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    serve_with_shutdown(relay, listener, std::future::pending()).await
}

pub async fn serve_with_shutdown(
    relay: Arc<Relay>,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let sweeper = spawn_sweeper(Arc::clone(&relay));
    let app = router(relay).into_make_service_with_connect_info::<SocketAddr>();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;
    sweeper.abort();
    result
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Relógio de parede em microssegundos (campo `sent_at_us` do `Pong`).
pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    // Falha do gerador do sistema é irrecuperável: sem aleatoriedade não há tokens seguros.
    if let Err(e) = getrandom::fill(&mut buf) {
        panic!("gerador de números aleatórios do sistema indisponível: {e}");
    }
    buf
}

/// Identificador aleatório em base64url com `n` bytes de entropia (n ≤ 32).
pub fn random_id(n: usize) -> String {
    let bytes = random_bytes::<32>();
    URL_SAFE_NO_PAD.encode(&bytes[..n.min(32)])
}

/// Comparação em tempo constante para segredos de tamanho arbitrário.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    use sha2::{Digest, Sha256};
    // Compara digests para não vazar o tamanho por tempo de execução.
    let (da, db) = (Sha256::digest(a), Sha256::digest(b));
    da.iter()
        .zip(db.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}
