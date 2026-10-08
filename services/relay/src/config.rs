//! Configuração do relay via variáveis de ambiente (ver `docs/DEPLOYMENT.md`).

use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// Tamanho mínimo do segredo HMAC.
pub const MIN_SECRET_LEN: usize = 32;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// Segredo HMAC-SHA256 dos tokens. Nunca é registrado em log.
    pub secret: Vec<u8>,
    /// Quando definido, `POST /api/rooms` exige `Authorization: Bearer <chave>`.
    pub create_room_key: Option<String>,
    /// Usa o último endereço de `X-Forwarded-For` (inserido pelo proxy reverso) como IP do cliente.
    pub trust_proxy: bool,
    pub max_rooms: usize,
    pub max_viewers_per_room: usize,
    pub max_publisher_kbps: u64,
    /// Mensagens na fila de cada espectador antes de descartar as mais antigas.
    pub viewer_queue: usize,
    /// Limite do cache do GOP atual (enviado a novos espectadores).
    pub gop_cache_bytes: usize,
    /// Quadros de vídeo há mais tempo que isso na fila do relay são descartados.
    pub stale_after: Duration,
    /// Tempo que a sala aguarda o transmissor reconectar.
    pub publisher_grace: Duration,
    pub room_ttl: Duration,
    /// Sala sem transmissor e sem espectadores por esse tempo é removida.
    pub room_idle_ttl: Duration,
    pub viewer_token_ttl: Duration,
    pub publisher_token_ttl: Duration,
    pub auth_timeout: Duration,
    pub client_idle_timeout: Duration,
    pub send_timeout: Duration,
    pub keyframe_request_interval: Duration,
    pub cleanup_interval: Duration,
    /// Requisições HTTP por IP: rajada e reposição por segundo.
    pub api_burst: f64,
    pub api_per_sec: f64,
    /// Conexões WebSocket por IP: rajada e reposição por segundo.
    pub ws_burst: f64,
    pub ws_per_sec: f64,
}

impl Config {
    /// Valores padrão com o segredo informado (base para testes e para `from_env`).
    pub fn with_secret(secret: Vec<u8>) -> Self {
        Self {
            bind: SocketAddr::from(([127, 0, 0, 1], 8788)),
            secret,
            create_room_key: None,
            trust_proxy: false,
            max_rooms: 1000,
            max_viewers_per_room: 50,
            max_publisher_kbps: 40_000,
            viewer_queue: 256,
            gop_cache_bytes: 8 * 1024 * 1024,
            stale_after: Duration::from_millis(500),
            publisher_grace: Duration::from_secs(30),
            room_ttl: Duration::from_secs(24 * 3600),
            room_idle_ttl: Duration::from_secs(3600),
            viewer_token_ttl: Duration::from_secs(12 * 3600),
            publisher_token_ttl: Duration::from_secs(24 * 3600),
            auth_timeout: Duration::from_secs(5),
            client_idle_timeout: Duration::from_secs(30),
            send_timeout: Duration::from_secs(10),
            keyframe_request_interval: Duration::from_millis(500),
            cleanup_interval: Duration::from_secs(15),
            api_burst: 30.0,
            api_per_sec: 2.0,
            ws_burst: 20.0,
            ws_per_sec: 2.0,
        }
    }

    pub fn from_env() -> Result<Self> {
        let secret = match std::env::var("VELLCAST_RELAY_SECRET") {
            Ok(s) if s.len() >= MIN_SECRET_LEN => s.into_bytes(),
            Ok(_) => bail!("VELLCAST_RELAY_SECRET precisa ter pelo menos {MIN_SECRET_LEN} bytes"),
            Err(_) if env_flag("VELLCAST_RELAY_INSECURE_DEV") => {
                tracing::warn!(
                    "modo de desenvolvimento: segredo aleatório; tokens perdem validade ao reiniciar"
                );
                crate::random_bytes::<32>().to_vec()
            }
            Err(_) => bail!(
                "defina VELLCAST_RELAY_SECRET (>= {MIN_SECRET_LEN} bytes) ou VELLCAST_RELAY_INSECURE_DEV=1 para desenvolvimento"
            ),
        };
        let mut c = Self::with_secret(secret);
        c.bind = env_or("VELLCAST_BIND", c.bind)?;
        c.create_room_key = std::env::var("VELLCAST_CREATE_ROOM_KEY")
            .ok()
            .filter(|k| !k.is_empty());
        c.trust_proxy = env_flag("VELLCAST_TRUST_PROXY");
        c.max_rooms = env_or("VELLCAST_MAX_ROOMS", c.max_rooms)?;
        c.max_viewers_per_room = env_or("VELLCAST_MAX_VIEWERS_PER_ROOM", c.max_viewers_per_room)?;
        c.max_publisher_kbps = env_or("VELLCAST_MAX_PUBLISHER_KBPS", c.max_publisher_kbps)?;
        c.publisher_grace = secs("VELLCAST_PUBLISHER_GRACE_SECS", c.publisher_grace)?;
        c.room_ttl = secs("VELLCAST_ROOM_TTL_SECS", c.room_ttl)?;
        c.room_idle_ttl = secs("VELLCAST_ROOM_IDLE_TTL_SECS", c.room_idle_ttl)?;
        c.viewer_token_ttl = secs("VELLCAST_VIEWER_TOKEN_TTL_SECS", c.viewer_token_ttl)?;
        Ok(c)
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes"))
}

fn env_or<T: FromStr>(name: &str, default: T) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    match std::env::var(name) {
        Ok(v) => v
            .parse()
            .map_err(|e| anyhow::anyhow!("{e}"))
            .with_context(|| format!("{name} inválido")),
        Err(_) => Ok(default),
    }
}

fn secs(name: &str, default: Duration) -> Result<Duration> {
    Ok(Duration::from_secs(env_or(name, default.as_secs())?))
}
