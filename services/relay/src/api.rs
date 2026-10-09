//! API HTTP de salas e convites.
//!
//! Tokens vão sempre no cabeçalho `Authorization: Bearer`, nunca na URL,
//! e nunca são registrados em log.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use vellcast_protocol::messages::{ErrorBody, ErrorCode};

use crate::room::{LiveRoom, RoomState};
use crate::store::{Invite, RoomRecord, StoreError, hash_invite};
use crate::token::{Claims, Role};
use crate::{Relay, constant_time_eq, metrics, random_id, unix_now};

const MAX_ROOM_NAME: usize = 64;
const MAX_INVITE_USES: u32 = 100;
const MIN_INVITE_TTL: u64 = 60;
const MAX_INVITE_TTL: u64 = 7 * 24 * 3600;

pub struct ApiError(StatusCode, ErrorCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            code: self.1,
            message: self.2.into(),
        };
        (self.0, Json(body)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

fn unauthorized() -> ApiError {
    ApiError(
        StatusCode::UNAUTHORIZED,
        ErrorCode::Unauthorized,
        "token inválido ou ausente",
    )
}

fn not_found() -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        ErrorCode::RoomNotFound,
        "sala não encontrada",
    )
}

fn bad_request(msg: &'static str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, ErrorCode::ProtocolError, msg)
}

fn rate_limit(relay: &Relay, peer: SocketAddr, headers: &HeaderMap) -> ApiResult<()> {
    if relay.api_limiter.check(relay.client_ip(peer, headers)) {
        Ok(())
    } else {
        metrics::inc(&relay.metrics.rate_limited);
        Err(ApiError(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::RateLimited,
            "muitas requisições",
        ))
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Valida o token e exige que ele pertença à sala e tenha um dos papéis.
fn authorize(relay: &Relay, headers: &HeaderMap, room: &str, roles: &[Role]) -> ApiResult<Claims> {
    let token = bearer(headers).ok_or_else(unauthorized)?;
    let claims = relay.signer.verify(token, unix_now()).map_err(|_| {
        metrics::inc(&relay.metrics.auth_failures);
        unauthorized()
    })?;
    if claims.room != room || relay.is_revoked(&claims) {
        metrics::inc(&relay.metrics.auth_failures);
        return Err(unauthorized());
    }
    if !roles.contains(&claims.role) {
        return Err(ApiError(
            StatusCode::FORBIDDEN,
            ErrorCode::Forbidden,
            "papel sem permissão",
        ));
    }
    Ok(claims)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub state: RoomState,
    pub viewers: usize,
    pub max_viewers: usize,
    pub created_at: u64,
    pub expires_at: u64,
}

fn view(record: &RoomRecord, live: &LiveRoom) -> RoomView {
    RoomView {
        id: record.id.clone(),
        name: record.name.clone(),
        owner_id: record.owner_id.clone(),
        state: live.state(),
        viewers: live.viewer_count(),
        max_viewers: record.max_viewers,
        created_at: record.created_at,
        expires_at: record.expires_at,
    }
}

fn lookup(relay: &Relay, id: &str) -> ApiResult<(RoomRecord, Arc<LiveRoom>)> {
    match (relay.store.get(id), relay.room(id)) {
        (Some(r), Some(l)) => Ok((r, l)),
        _ => Err(not_found()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRoom {
    pub name: String,
    pub max_viewers: Option<usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedRoom {
    pub room: RoomView,
    pub owner_token: String,
    pub publisher_token: String,
}

fn sanitize_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_ROOM_NAME || name.chars().any(char::is_control)
    {
        return Err(bad_request(
            "nome de sala inválido (1 a 64 caracteres, sem caracteres de controle)",
        ));
    }
    Ok(name.to_owned())
}

pub async fn create_room(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CreateRoom>,
) -> ApiResult<(StatusCode, Json<CreatedRoom>)> {
    rate_limit(&relay, peer, &headers)?;
    if let Some(key) = &relay.config.create_room_key {
        let given = bearer(&headers).unwrap_or_default();
        if !constant_time_eq(given.as_bytes(), key.as_bytes()) {
            metrics::inc(&relay.metrics.auth_failures);
            return Err(unauthorized());
        }
    }
    let entitlement = crate::entitlement::authorize(&relay, &headers).map_err(|denied| {
        metrics::inc(&relay.metrics.auth_failures);
        match denied {
            crate::entitlement::EntitlementDenied::Invalid => ApiError(
                StatusCode::UNAUTHORIZED,
                ErrorCode::Unauthorized,
                "entitlement da Vellcast Cloud ausente ou inválido",
            ),
            crate::entitlement::EntitlementDenied::RoomLimit => ApiError(
                StatusCode::FORBIDDEN,
                ErrorCode::Forbidden,
                "limite de salas do plano atingido",
            ),
        }
    })?;
    let name = sanitize_name(&body.name)?;
    if relay.store.len() >= relay.config.max_rooms {
        return Err(ApiError(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::RoomFull,
            "limite de salas do servidor atingido",
        ));
    }
    let max_viewers = body
        .max_viewers
        .unwrap_or(relay.config.max_viewers_per_room)
        .clamp(1, relay.config.max_viewers_per_room);
    // Modo hospedado: o plano limita os espectadores por sala.
    let max_viewers = entitlement.as_ref().map_or(max_viewers, |e| {
        max_viewers.min(e.max_viewers.max(1) as usize)
    });

    let now = unix_now();
    let record = RoomRecord {
        id: random_id(12),
        name,
        owner_id: random_id(9),
        created_at: now,
        expires_at: now + relay.config.room_ttl.as_secs(),
        max_viewers,
        invites: HashMap::new(),
        revoked: HashMap::new(),
        viewers_revoked_before: 0,
    };
    let c = &relay.config;
    let live = Arc::new(relay.new_live_room(&record.id, max_viewers));
    relay.store.insert(record.clone()).map_err(|_| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "falha ao criar sala",
        )
    })?;
    relay.insert_room(Arc::clone(&live));
    if let Some(e) = &entitlement {
        relay.account_rooms.record(&e.sub, &record.id);
    }
    metrics::inc(&relay.metrics.rooms_created);

    let (owner_token, _) = relay.signer.issue(&record.id, Role::Owner, c.room_ttl, now);
    let (publisher_token, _) = relay.signer.issue(
        &record.id,
        Role::Publisher,
        c.publisher_token_ttl.min(c.room_ttl),
        now,
    );
    tracing::info!(room = %record.id, max_viewers, "sala criada");
    Ok((
        StatusCode::CREATED,
        Json(CreatedRoom {
            room: view(&record, &live),
            owner_token,
            publisher_token,
        }),
    ))
}

pub async fn get_room(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<RoomView>> {
    rate_limit(&relay, peer, &headers)?;
    authorize(
        &relay,
        &headers,
        &id,
        &[Role::Owner, Role::Publisher, Role::Viewer],
    )?;
    let (record, live) = lookup(&relay, &id)?;
    Ok(Json(view(&record, &live)))
}

pub async fn delete_room(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<StatusCode> {
    rate_limit(&relay, peer, &headers)?;
    authorize(&relay, &headers, &id, &[Role::Owner])?;
    lookup(&relay, &id)?;
    relay.close_room(&id, "encerrada pelo dono");
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInvite {
    pub max_uses: Option<u32>,
    pub ttl_secs: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedInvite {
    /// Exibido uma única vez; o servidor guarda apenas o hash.
    pub code: String,
    pub max_uses: u32,
    pub expires_at: u64,
}

pub async fn create_invite(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Option<Json<CreateInvite>>,
) -> ApiResult<(StatusCode, Json<CreatedInvite>)> {
    rate_limit(&relay, peer, &headers)?;
    authorize(&relay, &headers, &id, &[Role::Owner])?;
    let (record, _) = lookup(&relay, &id)?;
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let max_uses = body.max_uses.unwrap_or(1).clamp(1, MAX_INVITE_USES);
    let ttl = body
        .ttl_secs
        .unwrap_or(3600)
        .clamp(MIN_INVITE_TTL, MAX_INVITE_TTL);
    let expires_at = (unix_now() + ttl).min(record.expires_at);
    let code = random_id(16);
    relay
        .store
        .add_invite(
            &id,
            hash_invite(&code),
            Invite {
                remaining_uses: max_uses,
                expires_at,
            },
        )
        .map_err(|e| match e {
            StoreError::TooManyInvites => ApiError(
                StatusCode::CONFLICT,
                ErrorCode::RateLimited,
                "limite de convites ativos atingido",
            ),
            _ => not_found(),
        })?;
    Ok((
        StatusCode::CREATED,
        Json(CreatedInvite {
            code,
            max_uses,
            expires_at,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JoinRoom {
    pub invite: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Joined {
    pub viewer_token: String,
    pub room: RoomView,
}

pub async fn join_room(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<JoinRoom>,
) -> ApiResult<Json<Joined>> {
    rate_limit(&relay, peer, &headers)?;
    if body.invite.len() > 128 {
        return Err(bad_request("convite inválido"));
    }
    let (record, live) = lookup(&relay, &id)?;
    let now = unix_now();
    relay
        .store
        .consume_invite(&id, &hash_invite(&body.invite), now)
        .map_err(|_| {
            metrics::inc(&relay.metrics.auth_failures);
            ApiError(
                StatusCode::FORBIDDEN,
                ErrorCode::Forbidden,
                "convite inválido ou expirado",
            )
        })?;
    let ttl = relay
        .config
        .viewer_token_ttl
        .min(Duration::from_secs(record.expires_at.saturating_sub(now)));
    let (viewer_token, _) = relay.signer.issue(&id, Role::Viewer, ttl, now);
    Ok(Json(Joined {
        viewer_token,
        room: view(&record, &live),
    }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublisherToken {
    pub publisher_token: String,
}

/// Emite um novo token de transmissor (ex.: outro computador do dono).
pub async fn publisher_token(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Json<PublisherToken>> {
    rate_limit(&relay, peer, &headers)?;
    authorize(&relay, &headers, &id, &[Role::Owner])?;
    let (record, _) = lookup(&relay, &id)?;
    let now = unix_now();
    let ttl = relay
        .config
        .publisher_token_ttl
        .min(Duration::from_secs(record.expires_at.saturating_sub(now)));
    let (publisher_token, _) = relay.signer.issue(&id, Role::Publisher, ttl, now);
    Ok(Json(PublisherToken { publisher_token }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Revoke {
    /// Token a revogar (ex.: um token de transmissor vazado). Precisa ser desta sala.
    pub token: Option<String>,
    /// Revoga todos os tokens de espectador emitidos até agora; quem estiver
    /// assistindo é desconectado e links já usados deixam de valer.
    #[serde(default)]
    pub all_viewers: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Revoked {
    pub revoked_token: bool,
    pub revoked_viewers_before: Option<u64>,
}

/// Revogação pelo dono: um token específico e/ou todos os espectadores.
/// Conexões ativas com tokens revogados são encerradas.
pub async fn revoke(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Revoke>,
) -> ApiResult<Json<Revoked>> {
    rate_limit(&relay, peer, &headers)?;
    let owner = authorize(&relay, &headers, &id, &[Role::Owner])?;
    let (_, live) = lookup(&relay, &id)?;
    if body.token.is_none() && !body.all_viewers {
        return Err(bad_request("informe `token` ou `allViewers`"));
    }
    let internal = |_| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            "falha ao registrar a revogação",
        )
    };
    let mut revoked_token = false;
    if let Some(token) = &body.token {
        if token.len() > crate::token::MAX_TOKEN_LEN {
            return Err(bad_request("token inválido"));
        }
        let target = relay
            .signer
            .verify(token, unix_now())
            .map_err(|_| bad_request("token inválido ou expirado"))?;
        if target.room != id {
            return Err(bad_request("o token não pertence a esta sala"));
        }
        if target.nonce == owner.nonce {
            return Err(bad_request("o dono não pode revogar o próprio token"));
        }
        relay
            .store
            .revoke_token(&id, &target.nonce, target.exp)
            .map_err(internal)?;
        revoked_token = true;
    }
    let mut revoked_viewers_before = None;
    if body.all_viewers {
        // +1: tokens emitidos neste mesmo segundo também ficam inválidos.
        let before = unix_now() + 1;
        relay
            .store
            .revoke_viewers_before(&id, before)
            .map_err(internal)?;
        revoked_viewers_before = Some(before);
    }
    live.notify_revocation();
    tracing::info!(room = %id, revoked_token, all_viewers = body.all_viewers, "tokens revogados");
    Ok(Json(Revoked {
        revoked_token,
        revoked_viewers_before,
    }))
}

pub async fn metrics(State(relay): State<Arc<Relay>>) -> Json<metrics::Snapshot> {
    Json(relay.metrics_snapshot())
}
