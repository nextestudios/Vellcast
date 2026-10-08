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
    fn fatal_codes() {
        assert!(ErrorCode::Unauthorized.is_fatal());
        assert!(!ErrorCode::RateLimited.is_fatal());
        assert!(!ErrorCode::Internal.is_fatal());
    }
}
