//! Payloads JSON das mensagens de controle.
//!
//! Mídia (vídeo/áudio) nunca passa por JSON; apenas configuração,
//! autenticação e erros, que são raros e pequenos.

use serde::{Deserialize, Serialize};

use crate::{EncodeError, Header, MessageType, encode};

/// Papel solicitado na conexão WebSocket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectRole {
    Publisher,
    Viewer,
}

/// Primeira mensagem do cliente (`Auth`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub token: String,
    pub role: ConnectRole,
}

/// Campos opcionais da mensagem `Auth`, lidos à parte de [`AuthRequest`]
/// para que clientes e relays antigos continuem compatíveis.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthProfile {
    /// Nome de exibição do transmissor na lista de participantes. O nome de
    /// espectadores vem do token (definido ao entrar com o convite) e este
    /// campo é ignorado para eles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Limite de caracteres de um nome de exibição de participante.
pub const MAX_PARTICIPANT_NAME_CHARS: usize = 32;

/// Normaliza um nome de exibição: remove caracteres de controle, apara
/// espaços e limita o tamanho. Devolve `None` se nada sobrar.
pub fn sanitize_participant_name(name: &str) -> Option<String> {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name: String = name
        .trim()
        .chars()
        .take(MAX_PARTICIPANT_NAME_CHARS)
        .collect();
    let name = name.trim_end();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Papel de um participante na lista de presença.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParticipantRole {
    Publisher,
    Viewer,
}

/// Um participante ativo. `id` é opaco e estável enquanto a sessão (token)
/// estiver conectada; não revela token nem convite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Participant {
    pub id: String,
    /// `None` para clientes que não informaram nome.
    pub name: Option<String>,
    pub role: ParticipantRole,
}

/// Payload de `Presence`: a lista completa (não incremental) de participantes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Presence {
    /// Total de participantes ativos; pode exceder `participants.len()` em
    /// salas muito grandes (a lista é limitada).
    pub count: usize,
    pub participants: Vec<Participant>,
}

/// Resposta `AuthOk`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthOk {
    pub room_id: String,
    pub role: ConnectRole,
    pub session_id: String,
    /// `true` quando a sala já tem um transmissor ativo.
    pub live: bool,
}

/// Códigos de erro estáveis (usados por clientes para decidir se reconectam).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Token ausente, inválido, adulterado ou expirado.
    Unauthorized,
    /// Token válido, mas sem permissão para o papel pedido.
    Forbidden,
    RoomNotFound,
    RoomFull,
    RoomClosed,
    PublisherExists,
    AuthTimeout,
    /// Conexão encerrada por inatividade (reconectar é seguro).
    IdleTimeout,
    MessageTooLarge,
    RateLimited,
    BitrateExceeded,
    ProtocolError,
    Internal,
}

impl ErrorCode {
    /// Erros após os quais reconectar com o mesmo token não adianta.
    pub fn is_fatal(self) -> bool {
        matches!(
            self,
            Self::Unauthorized
                | Self::Forbidden
                | Self::RoomNotFound
                | Self::RoomClosed
                | Self::PublisherExists
                | Self::ProtocolError
                | Self::MessageTooLarge
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
}

/// Configuração do decoder de vídeo (`VideoInit`), no formato do WebCodecs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInit {
    pub codec: String,
    pub coded_width: u32,
    pub coded_height: u32,
    pub framerate: u32,
}

/// Configuração do decoder de áudio (`AudioInit`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInit {
    pub codec: String,
    pub sample_rate: u32,
    pub number_of_channels: u8,
}

/// Quem compartilha a tela (`ShareChanged` e `PublisherReplaced`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareInfo {
    /// Nome exibido de quem compartilha; ausente quando o transmissor não informou.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `true` quando a vaga foi tomada de outro transmissor ativo (troca explícita).
    #[serde(default)]
    pub handoff: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum JsonMessageError {
    #[error(transparent)]
    Encode(#[from] EncodeError),
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
}

/// Serializa uma mensagem de controle com payload JSON.
pub fn encode_json<T: Serialize>(
    kind: MessageType,
    stream_id: u32,
    body: &T,
) -> Result<bytes::Bytes, JsonMessageError> {
    let payload = serde_json::to_vec(body)?;
    Ok(encode(&Header::new(kind, stream_id), &payload)?)
}

/// Mensagem `Error` pronta para envio.
pub fn error_message(code: ErrorCode, message: impl Into<String>) -> bytes::Bytes {
    let body = ErrorBody {
        code,
        message: message.into(),
    };
    // O corpo é pequeno e sempre serializável; o fallback só existe por segurança.
    encode_json(MessageType::Error, 0, &body)
        .unwrap_or_else(|_| bytes::Bytes::from_static(&[1, 0x32, 0, 0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MAX_MESSAGE_LEN, decode};

    #[test]
    fn auth_json_shape() {
        let req: AuthRequest =
            serde_json::from_str(r#"{"token":"abc","role":"publisher"}"#).expect("json");
        assert_eq!(req.role, ConnectRole::Publisher);
        assert!(serde_json::from_str::<AuthRequest>(r#"{"token":"abc","role":"admin"}"#).is_err());
    }

    #[test]
    fn error_message_roundtrip() {
        let msg = error_message(ErrorCode::RoomFull, "cheia");
        let (h, p) = decode(&msg, MAX_MESSAGE_LEN).expect("decode");
        assert_eq!(h.kind, MessageType::Error);
        let body: ErrorBody = serde_json::from_slice(&p).expect("json");
        assert_eq!(body.code, ErrorCode::RoomFull);
        assert_eq!(
            serde_json::to_string(&ErrorCode::PublisherExists).expect("json"),
            "\"publisher_exists\""
        );
    }

    #[test]
    fn auth_profile_is_optional_and_compatible() {
        let json = r#"{"token":"abc","role":"publisher","name":"Ana"}"#;
        // Relays antigos leem só `AuthRequest` e ignoram o campo extra.
        let req: AuthRequest = serde_json::from_str(json).expect("json");
        assert_eq!(req.token, "abc");
        let profile: AuthProfile = serde_json::from_str(json).expect("json");
        assert_eq!(profile.name.as_deref(), Some("Ana"));
        let none: AuthProfile =
            serde_json::from_str(r#"{"token":"abc","role":"viewer"}"#).expect("json");
        assert_eq!(none, AuthProfile::default());
    }

    #[test]
    fn participant_names_are_sanitized() {
        assert_eq!(
            sanitize_participant_name("  Ana \u{0007}\n"),
            Some("Ana".into())
        );
        assert_eq!(sanitize_participant_name("A\u{0000}na"), Some("Ana".into()));
        assert_eq!(sanitize_participant_name(" \t\n "), None);
        assert_eq!(sanitize_participant_name(""), None);
        let long = "x".repeat(100);
        assert_eq!(
            sanitize_participant_name(&long).map(|n| n.chars().count()),
            Some(MAX_PARTICIPANT_NAME_CHARS)
        );
        assert_eq!(
            sanitize_participant_name("<b>Zé</b>"),
            Some("<b>Zé</b>".into()),
            "marcação é texto; quem exibe é responsável por escapar"
        );
    }

    #[test]
    fn presence_json_shape() {
        let p = Presence {
            count: 1,
            participants: vec![Participant {
                id: "abc".into(),
                name: Some("Ana".into()),
                role: ParticipantRole::Publisher,
            }],
        };
        assert_eq!(
            serde_json::to_string(&p).expect("json"),
            r#"{"count":1,"participants":[{"id":"abc","name":"Ana","role":"publisher"}]}"#
        );
    }

    #[test]
    fn share_info_json_shape() {
        let info = ShareInfo {
            name: Some("Ana".into()),
            handoff: true,
        };
        let json = serde_json::to_string(&info).expect("json");
        assert_eq!(json, r#"{"name":"Ana","handoff":true}"#);
        // Campos ausentes têm padrão: transmissor sem nome, sem troca.
        let empty: ShareInfo = serde_json::from_str("{}").expect("json");
        assert_eq!(empty, ShareInfo::default());
        let msg = encode_json(MessageType::ShareChanged, 0, &info).expect("encode");
        let (h, p) = decode(&msg, MAX_MESSAGE_LEN).expect("decode");
        assert_eq!(h.kind, MessageType::ShareChanged);
        assert_eq!(serde_json::from_slice::<ShareInfo>(&p).expect("json"), info);
    }

    #[test]
    fn fatal_codes() {
        assert!(ErrorCode::Unauthorized.is_fatal());
        assert!(!ErrorCode::RateLimited.is_fatal());
        assert!(!ErrorCode::Internal.is_fatal());
    }
}
