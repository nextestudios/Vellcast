//! Cliente HTTP da API de salas do `vellcast-relay` (services/relay/src/api.rs).
//!
//! As chamadas são síncronas e só podem rodar em threads de fundo; a thread
//! da interface nunca espera a rede. Tokens vão sempre no cabeçalho
//! `Authorization` e nunca são registrados em log.

use std::time::Duration;

use serde::{Deserialize, Serialize};

const TIMEOUT: Duration = Duration::from_secs(10);
/// Usos de cada link de convite gerado pelo app (o relay limita a 100).
pub const INVITE_MAX_USES: u32 = 10;
/// Validade de cada link de convite (24 h; o relay limita à validade da sala).
pub const INVITE_TTL_SECS: u64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    Scheme,
    Invalid,
}

impl UrlError {
    pub fn message(&self, en: bool) -> &'static str {
        match (self, en) {
            (UrlError::Empty, false) => "informe o endereço do servidor",
            (UrlError::Empty, true) => "enter the server address",
            (UrlError::Scheme, false) => "o endereço precisa começar com http:// ou https://",
            (UrlError::Scheme, true) => "the address must start with http:// or https://",
            (UrlError::Invalid, false) => "endereço inválido",
            (UrlError::Invalid, true) => "invalid address",
        }
    }
}

/// Normaliza o endereço base do relay: `http(s)://host[:porta][/prefixo]`, sem barra final.
pub fn normalize_base(input: &str) -> Result<String, UrlError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(UrlError::Empty);
    }
    let lower = s.to_ascii_lowercase();
    let rest_start = if lower.starts_with("https://") {
        8
    } else if lower.starts_with("http://") {
        7
    } else {
        return Err(UrlError::Scheme);
    };
    let rest = s[rest_start..].trim_end_matches('/');
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty()
        || s.chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#' | '@' | '\\'))
    {
        return Err(UrlError::Invalid);
    }
    Ok(format!("{}{}", &lower[..rest_start], rest))
}

/// URL WebSocket do relay a partir da base: `http`→`ws`, `https`→`wss`, caminho `/ws`.
pub fn ws_url(base: &str) -> Result<String, UrlError> {
    let base = normalize_base(base)?;
    Ok(match base.strip_prefix("https://") {
        Some(rest) => format!("wss://{rest}/ws"),
        None => format!("ws://{}/ws", &base["http://".len()..]),
    })
}

/// Link para o espectador. O convite vai no fragmento (`#`), que o navegador
/// nunca envia ao servidor; assim ele não aparece em logs de acesso.
pub fn watch_link(base: &str, room_id: &str, invite: &str) -> Result<String, UrlError> {
    let base = normalize_base(base)?;
    Ok(format!("{base}/watch#room={room_id}&invite={invite}"))
}

/// `true` quando o endereço aponta para este próprio PC: links assim não
/// abrem em outros aparelhos.
pub fn is_loopback_base(base: &str) -> bool {
    let Ok(base) = normalize_base(base) else {
        return false;
    };
    let rest = base.split_once("://").map_or("", |(_, r)| r);
    let host = rest.split('/').next().unwrap_or_default();
    let host = host.rsplit_once(':').map_or(host, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    host.eq_ignore_ascii_case("localhost") || host.starts_with("127.") || host == "[::1]"
}

/// `true` quando o navegador do espectador terá um contexto seguro, exigido
/// pelo WebCodecs: HTTPS ou o próprio PC.
pub fn link_plays_in_browser(base: &str) -> bool {
    normalize_base(base).is_ok_and(|b| b.starts_with("https://")) || is_loopback_base(base)
}

/// Ids e códigos do relay são base64url; qualquer outra coisa é rejeitada
/// antes de entrar em uma URL.
pub fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoomView {
    pub id: String,
    pub name: String,
    /// `idle`, `live` ou `paused` (ver `RoomState` no relay).
    pub state: String,
    pub viewers: usize,
    pub max_viewers: usize,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedRoom {
    pub room: RoomView,
    pub owner_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedInvite {
    /// Exibido uma única vez pelo relay (ele guarda apenas o hash).
    pub code: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublisherToken {
    publisher_token: String,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    Url(UrlError),
    /// Servidor inacessível (desligado, endereço errado, firewall, TLS).
    Unreachable(String),
    /// A sala não existe mais no relay (expirou, foi encerrada ou o relay reiniciou).
    RoomGone,
    /// Chave de criação de sala ausente ou incorreta.
    Unauthorized,
    RateLimited,
    Rejected {
        status: u16,
        code: String,
    },
    BadResponse,
}

impl ApiError {
    /// Mensagem para o usuário no idioma da interface.
    pub fn user_message(&self, en: bool) -> String {
        let pick = |pt: &str, e: &str| if en { e.to_owned() } else { pt.to_owned() };
        match self {
            ApiError::Url(e) => {
                if en {
                    format!("Server address: {}.", e.message(true))
                } else {
                    format!("Endereço do servidor: {}.", e.message(false))
                }
            }
            ApiError::Unreachable(detail) => {
                let lower = detail.to_ascii_lowercase();
                let tls = lower.contains("certificate")
                    || lower.contains("unknownissuer")
                    || lower.contains("tls");
                match (tls, en) {
                    (true, false) => format!(
                        "O certificado do servidor não foi aceito ({detail}). Um antivírus ou proxy que inspeciona HTTPS pode estar interferindo."
                    ),
                    (true, true) => format!(
                        "The server certificate was not accepted ({detail}). An antivirus or proxy inspecting HTTPS may be interfering."
                    ),
                    (false, false) => format!(
                        "Não foi possível falar com o servidor ({detail}). Verifique o endereço em Configurações e a sua conexão."
                    ),
                    (false, true) => format!(
                        "Could not reach the server ({detail}). Check the address in Settings and your connection."
                    ),
                }
            }
            ApiError::RoomGone => pick(
                "Esta sala não existe mais no servidor (expirou, foi encerrada ou o servidor reiniciou). Ela foi removida da lista.",
                "This room no longer exists on the server (expired, closed or the server restarted). It was removed from the list.",
            ),
            ApiError::Unauthorized => pick(
                "O servidor recusou o acesso. Confira a chave de criação de salas em Configurações.",
                "The server refused access. Check the room creation key in Settings.",
            ),
            ApiError::RateLimited => pick(
                "Muitas tentativas seguidas. Aguarde alguns segundos.",
                "Too many attempts. Wait a few seconds.",
            ),
            ApiError::Rejected { status, code } => {
                if en {
                    format!("The server rejected the request (HTTP {status}, {code}).")
                } else {
                    format!("O servidor recusou o pedido (HTTP {status}, {code}).")
                }
            }
            ApiError::BadResponse => pick(
                "Resposta inesperada do servidor. Ele é um vellcast-relay?",
                "Unexpected server response. Is it a vellcast-relay?",
            ),
        }
    }
}

/// Falha de transporte (DNS, conexão, TLS): registra o motivo técnico no log.
fn unreachable(e: ureq::Error) -> ApiError {
    tracing::warn!(error = %e, "falha ao falar com o servidor");
    ApiError::Unreachable(e.to_string())
}

impl From<UrlError> for ApiError {
    fn from(e: UrlError) -> Self {
        ApiError::Url(e)
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        // Verificador de certificados do sistema (repositório do Windows), como o
        // navegador: antivírus que inspecionam HTTPS instalam uma raiz própria que
        // a lista embutida (webpki-roots) não conhece.
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .user_agent(concat!("vellcast-desktop/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Mapeia status HTTP + código de erro do relay.
pub fn classify(status: u16, code: Option<&str>) -> ApiError {
    match (status, code) {
        (404, _) | (_, Some("room_not_found" | "room_closed")) => ApiError::RoomGone,
        (401, _) | (_, Some("unauthorized")) => ApiError::Unauthorized,
        (429, _) | (_, Some("rate_limited")) => ApiError::RateLimited,
        (s, c) => ApiError::Rejected {
            status: s,
            code: c.unwrap_or("sem código").to_owned(),
        },
    }
}

fn finish<T: serde::de::DeserializeOwned>(
    result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<T, ApiError> {
    let mut resp = result.map_err(unreachable)?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return resp
            .body_mut()
            .read_json::<T>()
            .map_err(|_| ApiError::BadResponse);
    }
    let code = resp
        .body_mut()
        .read_json::<ErrorBody>()
        .ok()
        .map(|b| b.code);
    Err(classify(status, code.as_deref()))
}

fn checked_room(room_id: &str) -> Result<&str, ApiError> {
    if is_safe_id(room_id) {
        Ok(room_id)
    } else {
        Err(ApiError::RoomGone)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateRoomBody<'a> {
    name: &'a str,
}

pub fn create_room(base: &str, create_key: &str, name: &str) -> Result<CreatedRoom, ApiError> {
    crate::local_relay::ensure(base);
    let base = normalize_base(base)?;
    let mut req = agent().post(format!("{base}/api/rooms"));
    if !create_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", create_key.trim()));
    }
    finish(req.send_json(CreateRoomBody { name }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateInviteBody {
    max_uses: u32,
    ttl_secs: u64,
}

pub fn create_invite(
    base: &str,
    room_id: &str,
    owner_token: &str,
) -> Result<CreatedInvite, ApiError> {
    crate::local_relay::ensure(base);
    let base = normalize_base(base)?;
    let room = checked_room(room_id)?;
    finish(
        agent()
            .post(format!("{base}/api/rooms/{room}/invites"))
            .header("Authorization", format!("Bearer {owner_token}"))
            .send_json(CreateInviteBody {
                max_uses: INVITE_MAX_USES,
                ttl_secs: INVITE_TTL_SECS,
            }),
    )
}

/// Token de transmissor novo para a sala (emitido a cada início de transmissão).
pub fn publisher_token(base: &str, room_id: &str, owner_token: &str) -> Result<String, ApiError> {
    crate::local_relay::ensure(base);
    let base = normalize_base(base)?;
    let room = checked_room(room_id)?;
    finish::<PublisherToken>(
        agent()
            .post(format!("{base}/api/rooms/{room}/publisher-token"))
            .header("Authorization", format!("Bearer {owner_token}"))
            .send_empty(),
    )
    .map(|t| t.publisher_token)
}

pub fn get_room(base: &str, room_id: &str, owner_token: &str) -> Result<RoomView, ApiError> {
    crate::local_relay::ensure(base);
    let base = normalize_base(base)?;
    let room = checked_room(room_id)?;
    finish(
        agent()
            .get(format!("{base}/api/rooms/{room}"))
            .header("Authorization", format!("Bearer {owner_token}"))
            .call(),
    )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCode {
    /// O relay também devolve `expiresAt` (5 min); a interface só informa o prazo fixo.
    pub code: String,
}

/// Código de pareamento com a Discord Activity (5 min, uso único).
pub fn pairing_code(base: &str, room_id: &str, owner_token: &str) -> Result<PairingCode, ApiError> {
    crate::local_relay::ensure(base);
    let base = normalize_base(base)?;
    let room = checked_room(room_id)?;
    finish(
        agent()
            .post(format!("{base}/api/rooms/{room}/pairing"))
            .header("Authorization", format!("Bearer {owner_token}"))
            .send_empty(),
    )
}

/// Exibe o código em dois blocos para facilitar a leitura: `ABCD-EFGH`.
pub fn format_pairing_code(code: &str) -> String {
    let chars: Vec<char> = code.chars().collect();
    if chars.len() == 8 {
        format!(
            "{}-{}",
            chars[..4].iter().collect::<String>(),
            chars[4..].iter().collect::<String>()
        )
    } else {
        code.to_owned()
    }
}

/// `GET /healthz` do relay.
pub fn health(base: &str) -> Result<(), ApiError> {
    let base = normalize_base(base)?;
    let mut resp = agent()
        .get(format!("{base}/healthz"))
        .call()
        .map_err(unreachable)?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    if status == 200 && body.trim() == "ok" {
        Ok(())
    } else if status == 200 {
        Err(ApiError::BadResponse)
    } else {
        Err(classify(status, None))
    }
}

pub fn delete_room(base: &str, room_id: &str, owner_token: &str) -> Result<(), ApiError> {
    crate::local_relay::ensure(base);
    let base = normalize_base(base)?;
    let room = checked_room(room_id)?;
    let mut resp = agent()
        .delete(format!("{base}/api/rooms/{room}"))
        .header("Authorization", format!("Bearer {owner_token}"))
        .call()
        .map_err(unreachable)?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(());
    }
    let code = resp
        .body_mut()
        .read_json::<ErrorBody>()
        .ok()
        .map(|b| b.code);
    Err(classify(status, code.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_urls() {
        assert_eq!(
            normalize_base(" http://127.0.0.1:8788/ "),
            Ok("http://127.0.0.1:8788".into())
        );
        assert_eq!(
            normalize_base("HTTPS://Relay.Exemplo.com"),
            Ok("https://Relay.Exemplo.com".into())
        );
        assert_eq!(
            normalize_base("https://exemplo.com/vellcast//"),
            Ok("https://exemplo.com/vellcast".into())
        );
        assert_eq!(normalize_base(""), Err(UrlError::Empty));
        assert_eq!(normalize_base("relay.exemplo.com"), Err(UrlError::Scheme));
        assert_eq!(normalize_base("ftp://x"), Err(UrlError::Scheme));
        assert_eq!(normalize_base("http://"), Err(UrlError::Invalid));
        assert_eq!(normalize_base("http://a b"), Err(UrlError::Invalid));
        assert_eq!(normalize_base("http://x/?a=1"), Err(UrlError::Invalid));
        assert_eq!(normalize_base("http://user@x"), Err(UrlError::Invalid));
    }

    #[test]
    fn derives_websocket_url() {
        assert_eq!(
            ws_url("http://127.0.0.1:8788"),
            Ok("ws://127.0.0.1:8788/ws".into())
        );
        assert_eq!(
            ws_url("https://relay.exemplo.com/"),
            Ok("wss://relay.exemplo.com/ws".into())
        );
        assert_eq!(
            ws_url("https://exemplo.com/vellcast"),
            Ok("wss://exemplo.com/vellcast/ws".into())
        );
        assert_eq!(ws_url("exemplo.com"), Err(UrlError::Scheme));
    }

    #[test]
    fn builds_watch_link_with_invite_in_fragment() {
        let link = watch_link("https://relay.exemplo.com/", "abc_DEF-1", "code123").expect("link");
        assert_eq!(
            link,
            "https://relay.exemplo.com/watch#room=abc_DEF-1&invite=code123"
        );
        let (before_fragment, _) = link.split_once('#').expect("fragmento");
        assert!(
            !before_fragment.contains("code123"),
            "convite fora do fragmento"
        );
    }

    #[test]
    fn detects_loopback_and_secure_link_bases() {
        assert!(is_loopback_base("http://127.0.0.1:8788"));
        assert!(is_loopback_base("http://localhost"));
        assert!(is_loopback_base("http://[::1]:8788"));
        assert!(!is_loopback_base("http://192.168.0.10:8788"));
        assert!(!is_loopback_base("https://relay.exemplo.com"));
        assert!(link_plays_in_browser("https://relay.exemplo.com"));
        assert!(link_plays_in_browser("http://127.0.0.1:8788"));
        // HTTP em IP de rede não é contexto seguro: o WebCodecs fica indisponível.
        assert!(!link_plays_in_browser("http://192.168.0.10:8788"));
    }

    #[test]
    fn formats_pairing_code_in_two_blocks() {
        assert_eq!(format_pairing_code("ABCD2345"), "ABCD-2345");
        assert_eq!(format_pairing_code("ABC"), "ABC");
    }

    #[test]
    fn rejects_unsafe_ids() {
        assert!(is_safe_id("smwRDqaAw4ALng3j"));
        assert!(!is_safe_id(""));
        assert!(!is_safe_id("../admin"));
        assert!(!is_safe_id("a b"));
        assert!(!is_safe_id(&"a".repeat(200)));
    }

    #[test]
    fn classifies_relay_errors() {
        assert_eq!(classify(404, None), ApiError::RoomGone);
        assert_eq!(classify(400, Some("room_closed")), ApiError::RoomGone);
        assert_eq!(classify(401, Some("unauthorized")), ApiError::Unauthorized);
        assert_eq!(classify(429, None), ApiError::RateLimited);
        assert_eq!(
            classify(503, Some("room_full")),
            ApiError::Rejected {
                status: 503,
                code: "room_full".into()
            }
        );
    }

    #[test]
    fn unreachable_server_is_reported_without_panicking() {
        // Porta 9 (discard) em loopback: conexão recusada na hora.
        let err = create_room("http://127.0.0.1:9", "", "Sala").expect_err("deve falhar");
        assert!(matches!(err, ApiError::Unreachable(_)), "{err:?}");
        assert!(!err.user_message(false).is_empty());
        assert!(!err.user_message(true).is_empty());
    }

    /// Sobe um `vellcast-relay` real e percorre o fluxo completo do app.
    #[test]
    fn full_room_flow_against_real_relay() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("runtime");
        let mut config = vellcast_relay::config::Config::with_secret(
            b"desktop-test-secret-desktop-test-se".to_vec(),
        );
        config.create_room_key = Some("chave-de-teste".into());
        let relay = vellcast_relay::Relay::new(config);
        let listener = rt
            .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
            .expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        rt.spawn(vellcast_relay::serve(relay, listener));

        health(&base).expect("healthz");
        assert_eq!(
            create_room(&base, "", "Sem chave").err(),
            Some(ApiError::Unauthorized)
        );
        let created = create_room(&base, "chave-de-teste", "Sexta à noite").expect("criar sala");
        assert_eq!(created.room.name, "Sexta à noite");
        assert!(is_safe_id(&created.room.id));
        let id = created.room.id.clone();
        let owner = created.owner_token;

        let invite = create_invite(&base, &id, &owner).expect("convite");
        let link = watch_link(&base, &id, &invite.code).expect("link");
        assert!(link.starts_with(&format!("{base}/watch#room={id}&invite=")));

        let publisher = publisher_token(&base, &id, &owner).expect("token de transmissor");
        assert!(!publisher.is_empty());
        let view = get_room(&base, &id, &owner).expect("sala");
        assert_eq!((view.viewers, view.state.as_str()), (0, "idle"));

        // Token de outra sala/forjado é recusado.
        assert!(get_room(&base, &id, "v1.forjado.assinatura").is_err());

        delete_room(&base, &id, &owner).expect("encerrar");
        assert_eq!(get_room(&base, &id, &owner).err(), Some(ApiError::RoomGone));
        assert_eq!(
            create_invite(&base, &id, &owner).err(),
            Some(ApiError::RoomGone)
        );
        rt.shutdown_background();
    }
}
