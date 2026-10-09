//! Integração com a Discord Activity: OAuth2 no backend (M-014) e pareamento
//! entre o app desktop e a instância da Activity (M-015).
//!
//! Fluxo (ver `docs/DISCORD_INTEGRATION.md`):
//! 1. A Activity chama `authorize` no SDK e envia o `code` para
//!    `POST /api/discord/token`.
//! 2. O relay troca o código por um access token **no servidor** (o client
//!    secret nunca sai daqui), consulta `users/@me` com esse token e emite uma
//!    sessão Vellcast curta vinculada ao ID do usuário. A identidade nunca vem
//!    do frontend.
//! 3. O dono da sala gera um código de pareamento no desktop
//!    (`POST /api/rooms/{id}/pairing`); na Activity, o usuário digita o código
//!    (`POST /api/discord/pair`), e a instância passa a listar a transmissão
//!    (`GET /api/discord/instances/{instance}/streams`).
//!
//! Desligado quando `VELLCAST_DISCORD_CLIENT_ID`/`VELLCAST_DISCORD_CLIENT_SECRET`
//! não estão definidos (as rotas respondem 503). O cliente HTTP é o `ureq`
//! (síncrono, rustls, pequeno), executado em `spawn_blocking`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use vellcast_protocol::messages::{ErrorBody, ErrorCode};

use crate::pairing::{PairError, PairingStore, valid_instance_id};
use crate::ratelimit::IpRateLimiter;
use crate::token::Role;
use crate::{Relay, metrics, unix_now};

/// Prefixo do campo `room` nas sessões Discord: nunca colide com IDs de sala
/// (base64url aleatório), então uma sessão não serve como token de sala.
const SESSION_PREFIX: &str = "discord-user:";
const DEFAULT_API_BASE: &str = "https://discord.com/api/v10";
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CODE_LEN: usize = 512;

#[derive(Clone)]
pub struct DiscordConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Base da API do Discord (configurável para testes).
    pub api_base: String,
    /// Token de bot opcional: confirma que o usuário participa da instância.
    pub bot_token: Option<String>,
    pub session_ttl: Duration,
}

impl std::fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("client_id", &self.client_id)
            .field("api_base", &self.api_base)
            .field("bot_token", &self.bot_token.as_ref().map(|_| "***"))
            .finish_non_exhaustive()
    }
}

impl DiscordConfig {
    /// Lê `VELLCAST_DISCORD_CLIENT_ID`, `VELLCAST_DISCORD_CLIENT_SECRET`,
    /// `VELLCAST_DISCORD_BOT_TOKEN` (opcional) e `VELLCAST_DISCORD_API_BASE` (opcional).
    pub fn from_env() -> Option<Self> {
        let get = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Some(Self {
            client_id: get("VELLCAST_DISCORD_CLIENT_ID")?,
            client_secret: get("VELLCAST_DISCORD_CLIENT_SECRET")?,
            api_base: get("VELLCAST_DISCORD_API_BASE")
                .unwrap_or_else(|| DEFAULT_API_BASE.into())
                .trim_end_matches('/')
                .to_owned(),
            bot_token: get("VELLCAST_DISCORD_BOT_TOKEN"),
            session_ttl: Duration::from_secs(2 * 3600),
        })
    }
}

pub struct DiscordState {
    relay: Arc<Relay>,
    config: Option<DiscordConfig>,
    pub pairing: PairingStore,
    /// Limite mais estrito para tentativas de pareamento e troca de código.
    attempts: IpRateLimiter,
}

/// Rotas da integração Discord, a serem mescladas ao router principal.
pub fn router(relay: Arc<Relay>, config: Option<DiscordConfig>) -> Router {
    if config.is_some() {
        tracing::info!("integração com a Discord Activity habilitada");
    }
    let state = Arc::new(DiscordState {
        relay,
        config,
        pairing: PairingStore::default(),
        attempts: IpRateLimiter::new(10.0, 0.5),
    });
    let sweeper = Arc::clone(&state);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            sweeper.pairing.sweep(unix_now());
        }
    });
    Router::new()
        .route("/api/discord/token", post(exchange_token))
        .route("/api/discord/pair", post(pair))
        .route("/api/discord/instances/{instance}/streams", get(streams))
        .route("/api/rooms/{id}/pairing", post(create_pairing_code))
        .with_state(state)
}

pub struct DiscordError(StatusCode, ErrorCode, &'static str);

impl IntoResponse for DiscordError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(ErrorBody {
                code: self.1,
                message: self.2.into(),
            }),
        )
            .into_response()
    }
}

type DResult<T> = Result<T, DiscordError>;

fn unauthorized() -> DiscordError {
    DiscordError(
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthorized,
        "sessão inválida ou ausente",
    )
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn limit(state: &DiscordState, peer: SocketAddr, headers: &HeaderMap, strict: bool) -> DResult<()> {
    let ip = state.relay.client_ip(peer, headers);
    let ok = state.relay.api_limiter.check(ip) && (!strict || state.attempts.check(ip));
    if ok {
        Ok(())
    } else {
        metrics::inc(&state.relay.metrics.rate_limited);
        Err(DiscordError(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "muitas tentativas",
        ))
    }
}

fn config(state: &DiscordState) -> DResult<&DiscordConfig> {
    state.config.as_ref().ok_or(DiscordError(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Forbidden,
        "integração com o Discord não configurada neste servidor",
    ))
}

/// ID do usuário Discord da sessão Vellcast.
fn session_user(state: &DiscordState, headers: &HeaderMap) -> DResult<String> {
    let token = bearer(headers).ok_or_else(unauthorized)?;
    let claims = state.relay.signer.verify(token, unix_now()).map_err(|_| {
        metrics::inc(&state.relay.metrics.auth_failures);
        unauthorized()
    })?;
    claims
        .room
        .strip_prefix(SESSION_PREFIX)
        .map(str::to_owned)
        .ok_or_else(unauthorized)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenRequest {
    pub code: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenResponse {
    /// Repassado ao `authenticate` do SDK.
    pub access_token: String,
    /// Sessão Vellcast (Bearer) para as rotas de pareamento e listagem.
    pub session_token: String,
    pub user: DiscordUser,
}

#[derive(Deserialize)]
struct OAuthToken {
    access_token: String,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .http_status_as_error(false)
        .user_agent(concat!("vellcast-relay/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Troca o código OAuth2 e identifica o usuário (bloqueante: rodar em `spawn_blocking`).
fn exchange_blocking(
    cfg: &DiscordConfig,
    code: &str,
) -> Result<(String, DiscordUser), &'static str> {
    let agent = agent();
    let mut resp = agent
        .post(format!("{}/oauth2/token", cfg.api_base))
        .send_form([
            ("client_id", cfg.client_id.as_str()),
            ("client_secret", cfg.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code),
        ])
        .map_err(|_| "Discord indisponível")?;
    if !resp.status().is_success() {
        return Err("código OAuth2 recusado pelo Discord");
    }
    let token: OAuthToken = resp
        .body_mut()
        .read_json()
        .map_err(|_| "resposta inválida do Discord")?;
    let mut me = agent
        .get(format!("{}/users/@me", cfg.api_base))
        .header("Authorization", format!("Bearer {}", token.access_token))
        .call()
        .map_err(|_| "Discord indisponível")?;
    if !me.status().is_success() {
        return Err("token do Discord recusado");
    }
    let user: DiscordUser = me
        .body_mut()
        .read_json()
        .map_err(|_| "resposta inválida do Discord")?;
    Ok((token.access_token, user))
}

#[derive(Deserialize)]
struct ActivityInstance {
    #[serde(default)]
    users: Vec<String>,
}

/// Com token de bot, confirma que o usuário está na instância da Activity.
fn user_in_instance(cfg: &DiscordConfig, instance: &str, user: &str) -> Result<bool, &'static str> {
    let Some(bot) = &cfg.bot_token else {
        return Ok(true);
    };
    let mut resp = agent()
        .get(format!(
            "{}/applications/{}/activity-instances/{instance}",
            cfg.api_base, cfg.client_id
        ))
        .header("Authorization", format!("Bot {bot}"))
        .call()
        .map_err(|_| "Discord indisponível")?;
    if !resp.status().is_success() {
        return Ok(false);
    }
    let inst: ActivityInstance = resp
        .body_mut()
        .read_json()
        .map_err(|_| "resposta inválida do Discord")?;
    Ok(inst.users.iter().any(|u| u == user))
}

pub async fn exchange_token(
    State(state): State<Arc<DiscordState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<TokenRequest>,
) -> DResult<Json<TokenResponse>> {
    limit(&state, peer, &headers, true)?;
    let cfg = config(&state)?.clone();
    if body.code.is_empty() || body.code.len() > MAX_CODE_LEN {
        return Err(DiscordError(
            StatusCode::BAD_REQUEST,
            ErrorCode::ProtocolError,
            "código inválido",
        ));
    }
    let result = tokio::task::spawn_blocking(move || exchange_blocking(&cfg, &body.code))
        .await
        .map_err(|_| {
            DiscordError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                "falha interna",
            )
        })?;
    let (access_token, user) = result.map_err(|msg| {
        metrics::inc(&state.relay.metrics.auth_failures);
        tracing::warn!(reason = msg, "falha na autenticação com o Discord");
        DiscordError(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized, msg)
    })?;
    let ttl = state
        .config
        .as_ref()
        .map_or(Duration::from_secs(3600), |c| c.session_ttl);
    let (session_token, _) = state.relay.signer.issue(
        &format!("{SESSION_PREFIX}{}", user.id),
        Role::Viewer,
        ttl,
        unix_now(),
    );
    tracing::info!(user = %user.id, "sessão Discord criada");
    Ok(Json(TokenResponse {
        access_token,
        session_token,
        user,
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    pub code: String,
    pub expires_at: u64,
}

/// Dono da sala gera um código de pareamento (exibido no desktop).
pub async fn create_pairing_code(
    State(state): State<Arc<DiscordState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> DResult<(StatusCode, Json<PairingCode>)> {
    limit(&state, peer, &headers, false)?;
    let token = bearer(&headers).ok_or_else(unauthorized)?;
    let now = unix_now();
    let claims = state
        .relay
        .signer
        .verify(token, now)
        .map_err(|_| unauthorized())?;
    if claims.room != id || claims.role != Role::Owner {
        metrics::inc(&state.relay.metrics.auth_failures);
        return Err(DiscordError(
            StatusCode::FORBIDDEN,
            ErrorCode::Forbidden,
            "apenas o dono da sala",
        ));
    }
    let record = state.relay.store.get(&id).ok_or(DiscordError(
        StatusCode::NOT_FOUND,
        ErrorCode::RoomNotFound,
        "sala não encontrada",
    ))?;
    let (code, expires_at) = state
        .pairing
        .create_code(&id, now, record.expires_at)
        .map_err(|_| {
            DiscordError(
                StatusCode::CONFLICT,
                ErrorCode::RateLimited,
                "limite de códigos ativos",
            )
        })?;
    Ok((StatusCode::CREATED, Json(PairingCode { code, expires_at })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairRequest {
    pub code: String,
    pub instance_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamEntry {
    pub room_id: String,
    pub name: String,
    pub state: crate::room::RoomState,
    pub viewers: usize,
    /// Token de espectador para esta sala (curto; pedir de novo ao expirar).
    pub viewer_token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamList {
    pub streams: Vec<StreamEntry>,
}

fn entry(relay: &Relay, room_id: &str, now: u64) -> Option<StreamEntry> {
    let record = relay.store.get(room_id)?;
    let live = relay.room(room_id)?;
    let ttl = relay
        .config
        .viewer_token_ttl
        .min(Duration::from_secs(record.expires_at.saturating_sub(now)));
    let (viewer_token, _) = relay.signer.issue(room_id, Role::Viewer, ttl, now);
    Some(StreamEntry {
        room_id: record.id,
        name: record.name,
        state: live.state(),
        viewers: live.viewer_count(),
        viewer_token,
    })
}

async fn check_instance(state: &DiscordState, instance: &str, user: &str) -> DResult<()> {
    if !valid_instance_id(instance) {
        return Err(DiscordError(
            StatusCode::BAD_REQUEST,
            ErrorCode::ProtocolError,
            "instância inválida",
        ));
    }
    let cfg = config(state)?.clone();
    let (instance, user) = (instance.to_owned(), user.to_owned());
    let member = tokio::task::spawn_blocking(move || user_in_instance(&cfg, &instance, &user))
        .await
        .map_err(|_| {
            DiscordError(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                "falha interna",
            )
        })?
        .map_err(|msg| DiscordError(StatusCode::BAD_GATEWAY, ErrorCode::Internal, msg))?;
    if member {
        Ok(())
    } else {
        Err(DiscordError(
            StatusCode::FORBIDDEN,
            ErrorCode::Forbidden,
            "usuário fora desta instância da Activity",
        ))
    }
}

/// Participante da Activity digita o código mostrado no desktop.
pub async fn pair(
    State(state): State<Arc<DiscordState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PairRequest>,
) -> DResult<Json<StreamList>> {
    limit(&state, peer, &headers, true)?;
    config(&state)?;
    let user = session_user(&state, &headers)?;
    check_instance(&state, &body.instance_id, &user).await?;
    let now = unix_now();
    let relay = Arc::clone(&state.relay);
    state
        .pairing
        .pair(&body.code, &body.instance_id, now, |room| {
            relay
                .room(room)
                .and(relay.store.get(room))
                .map(|r| r.expires_at)
        })
        .map_err(|e| {
            metrics::inc(&state.relay.metrics.auth_failures);
            match e {
                PairError::TooManyRooms => DiscordError(
                    StatusCode::CONFLICT,
                    ErrorCode::RoomFull,
                    "muitas salas vinculadas",
                ),
                PairError::InvalidInstance => DiscordError(
                    StatusCode::BAD_REQUEST,
                    ErrorCode::ProtocolError,
                    "instância inválida",
                ),
                _ => DiscordError(
                    StatusCode::FORBIDDEN,
                    ErrorCode::Forbidden,
                    "código inválido ou expirado",
                ),
            }
        })?;
    tracing::info!(user = %user, "instância da Activity pareada");
    Ok(Json(list(&state, &body.instance_id, now)))
}

fn list(state: &DiscordState, instance: &str, now: u64) -> StreamList {
    StreamList {
        streams: state
            .pairing
            .rooms_for(instance, now)
            .iter()
            .filter_map(|r| entry(&state.relay, r, now))
            .collect(),
    }
}

/// Transmissões vinculadas à instância da Activity.
pub async fn streams(
    State(state): State<Arc<DiscordState>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(instance): Path<String>,
    headers: HeaderMap,
) -> DResult<Json<StreamList>> {
    limit(&state, peer, &headers, false)?;
    config(&state)?;
    let user = session_user(&state, &headers)?;
    check_instance(&state, &instance, &user).await?;
    Ok(Json(list(&state, &instance, unix_now())))
}
