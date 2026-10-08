//! Tokens assinados com HMAC-SHA256.
//!
//! Formato: `v1.<payload base64url>.<assinatura base64url>`, em que o payload
//! é JSON com sala, papel, expiração e nonce. A verificação da assinatura é
//! feita em tempo constante (`Mac::verify_slice`).

use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const PREFIX: &str = "v1";
/// Tokens maiores que isso são rejeitados antes de qualquer decodificação.
pub const MAX_TOKEN_LEN: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Dono da sala: gerencia convites, pode transmitir e assistir.
    Owner,
    Publisher,
    Viewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub room: String,
    pub role: Role,
    /// Expiração (Unix, segundos).
    pub exp: u64,
    /// Identificador único do token (identifica a sessão do transmissor em reconexões).
    pub nonce: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("token malformado")]
    Malformed,
    #[error("assinatura inválida")]
    BadSignature,
    #[error("token expirado")]
    Expired,
}

pub struct TokenSigner {
    key: Vec<u8>,
}

impl TokenSigner {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            key: secret.to_vec(),
        }
    }

    fn mac(&self) -> HmacSha256 {
        // HMAC aceita chaves de qualquer tamanho; o erro é inalcançável.
        <HmacSha256 as KeyInit>::new_from_slice(&self.key).unwrap_or_else(|_| unreachable!())
    }

    pub fn issue(&self, room: &str, role: Role, ttl: Duration, now: u64) -> (String, Claims) {
        let claims = Claims {
            room: room.to_owned(),
            role,
            exp: now + ttl.as_secs(),
            nonce: crate::random_id(12),
        };
        (self.sign(&claims), claims)
    }

    pub fn sign(&self, claims: &Claims) -> String {
        let json = serde_json::to_vec(claims).unwrap_or_default();
        let body = format!("{PREFIX}.{}", URL_SAFE_NO_PAD.encode(json));
        let mut mac = self.mac();
        mac.update(body.as_bytes());
        let sig = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{body}.{sig}")
    }

    pub fn verify(&self, token: &str, now: u64) -> Result<Claims, TokenError> {
        if token.len() > MAX_TOKEN_LEN {
            return Err(TokenError::Malformed);
        }
        let (body, sig) = token.rsplit_once('.').ok_or(TokenError::Malformed)?;
        let (prefix, payload) = body.split_once('.').ok_or(TokenError::Malformed)?;
        if prefix != PREFIX {
            return Err(TokenError::Malformed);
        }
        let sig = URL_SAFE_NO_PAD
            .decode(sig)
            .map_err(|_| TokenError::Malformed)?;
        let mut mac = self.mac();
        mac.update(body.as_bytes());
        mac.verify_slice(&sig)
            .map_err(|_| TokenError::BadSignature)?;
        // Só interpreta o payload depois de autenticado.
        let json = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| TokenError::Malformed)?;
        let claims: Claims = serde_json::from_slice(&json).map_err(|_| TokenError::Malformed)?;
        if claims.exp <= now {
            return Err(TokenError::Expired);
        }
        Ok(claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn signer() -> TokenSigner {
        TokenSigner::new(b"0123456789abcdef0123456789abcdef")
    }

    #[test]
    fn valid_token_roundtrip() {
        let s = signer();
        let (token, claims) = s.issue("room1", Role::Viewer, Duration::from_secs(60), NOW);
        assert_eq!(s.verify(&token, NOW + 59), Ok(claims));
    }

    #[test]
    fn expired_token() {
        let s = signer();
        let (token, _) = s.issue("room1", Role::Viewer, Duration::from_secs(60), NOW);
        assert_eq!(s.verify(&token, NOW + 60), Err(TokenError::Expired));
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let s = signer();
        let (token, mut claims) = s.issue("room1", Role::Viewer, Duration::from_secs(60), NOW);
        // Troca o papel mantendo a assinatura original.
        claims.role = Role::Owner;
        let forged_body = s.sign(&claims);
        let forged_payload = forged_body.split('.').nth(1).unwrap_or_default();
        let original_sig = token.rsplit('.').next().unwrap_or_default();
        let forged = format!("v1.{forged_payload}.{original_sig}");
        assert_eq!(s.verify(&forged, NOW), Err(TokenError::BadSignature));
    }

    #[test]
    fn other_secret_is_rejected() {
        let (token, _) = signer().issue("room1", Role::Owner, Duration::from_secs(60), NOW);
        let other = TokenSigner::new(b"another-secret-another-secret-xx");
        assert_eq!(other.verify(&token, NOW), Err(TokenError::BadSignature));
    }

    #[test]
    fn malformed_tokens() {
        let s = signer();
        for t in ["", "v1", "v1.abc", "v2.abc.def", "v1.@@@.###"] {
            assert!(s.verify(t, NOW).is_err(), "{t}");
        }
        assert_eq!(
            s.verify(&"a".repeat(MAX_TOKEN_LEN + 1), NOW),
            Err(TokenError::Malformed)
        );
    }

    #[test]
    fn nonces_are_unique() {
        let s = signer();
        let (_, a) = s.issue("r", Role::Publisher, Duration::from_secs(1), NOW);
        let (_, b) = s.issue("r", Role::Publisher, Duration::from_secs(1), NOW);
        assert_ne!(a.nonce, b.nonce);
    }
}
