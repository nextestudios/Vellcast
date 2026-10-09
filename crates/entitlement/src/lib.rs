//! Tokens de entitlement da Vellcast Cloud.
//!
//! A cloud-api emite um token curto, assinado com HMAC-SHA256, com os limites do
//! plano da conta. O relay hospedado, configurado com a mesma chave, verifica o
//! token ao criar salas e aplica os limites. Um relay self-hosted não usa nada
//! disto (W-013): sem chave configurada, nenhum token é exigido.
//!
//! Formato: `ve1.<payload base64url>.<assinatura base64url>`; o payload é JSON.
//! A resolução máxima nunca é usada para limitar o plano gratuito abaixo de
//! 1080p60 (W-014); ela existe só para liberar resoluções maiores (C-001/C-002).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const PREFIX: &str = "ve1";
/// Chaves menores que isso são recusadas.
pub const MIN_KEY_LEN: usize = 32;
/// Tokens maiores que isso são recusados antes de qualquer decodificação.
pub const MAX_TOKEN_LEN: usize = 2048;

/// Limites do plano carregados no token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// ID da conta na cloud-api.
    pub sub: String,
    /// `free`, `pro` ou `community`.
    pub plan: String,
    pub max_viewers: u32,
    pub max_rooms: u32,
    /// Altura máxima de transmissão (nunca menor que 1080 no plano gratuito).
    pub max_height: u32,
    pub max_fps: u32,
    /// Expiração (Unix, segundos).
    pub exp: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EntitlementError {
    #[error("chave de entitlement curta demais")]
    WeakKey,
    #[error("token de entitlement malformado")]
    Malformed,
    #[error("assinatura do entitlement inválida")]
    BadSignature,
    #[error("entitlement expirado")]
    Expired,
}

fn mac(key: &[u8]) -> Result<HmacSha256, EntitlementError> {
    if key.len() < MIN_KEY_LEN {
        return Err(EntitlementError::WeakKey);
    }
    <HmacSha256 as KeyInit>::new_from_slice(key).map_err(|_| EntitlementError::WeakKey)
}

/// Assina `claims` com `key`.
pub fn sign(key: &[u8], claims: &Claims) -> Result<String, EntitlementError> {
    let json = serde_json::to_vec(claims).map_err(|_| EntitlementError::Malformed)?;
    let body = format!("{PREFIX}.{}", URL_SAFE_NO_PAD.encode(json));
    let mut m = mac(key)?;
    m.update(body.as_bytes());
    Ok(format!(
        "{body}.{}",
        URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
    ))
}

/// Verifica a assinatura (tempo constante) e a expiração.
pub fn verify(key: &[u8], token: &str, now: u64) -> Result<Claims, EntitlementError> {
    if token.len() > MAX_TOKEN_LEN {
        return Err(EntitlementError::Malformed);
    }
    let (body, sig) = token.rsplit_once('.').ok_or(EntitlementError::Malformed)?;
    let (prefix, payload) = body.split_once('.').ok_or(EntitlementError::Malformed)?;
    if prefix != PREFIX {
        return Err(EntitlementError::Malformed);
    }
    let sig = URL_SAFE_NO_PAD
        .decode(sig)
        .map_err(|_| EntitlementError::Malformed)?;
    let mut m = mac(key)?;
    m.update(body.as_bytes());
    m.verify_slice(&sig)
        .map_err(|_| EntitlementError::BadSignature)?;
    let json = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| EntitlementError::Malformed)?;
    let claims: Claims = serde_json::from_slice(&json).map_err(|_| EntitlementError::Malformed)?;
    if claims.exp <= now {
        return Err(EntitlementError::Expired);
    }
    Ok(claims)
}

#[cfg(test)]
mod tests {

    /// Vetor fixo compartilhado com nextestudios/vellcast-cloud (cópia deste
    /// crate): se o formato mudar em um repositório só, este teste quebra.
    #[test]
    fn token_format_matches_relay() {
        let claims = Claims {
            sub: "conta-1".into(),
            plan: "pro".into(),
            max_viewers: 50,
            max_rooms: 5,
            max_height: 1440,
            max_fps: 60,
            exp: 2_000_000_000,
        };
        let key = b"chave-de-teste-com-32-bytes-ok!!";
        let expected = "ve1.eyJzdWIiOiJjb250YS0xIiwicGxhbiI6InBybyIsIm1heF92aWV3ZXJzIjo1MCwibWF4X3Jvb21zIjo1LCJtYXhfaGVpZ2h0IjoxNDQwLCJtYXhfZnBzIjo2MCwiZXhwIjoyMDAwMDAwMDAwfQ.L35AewiTfoAS9ZcbnJZ0ItZ0Y2zKgT_Jof8LOnAuhTY";
        assert_eq!(sign(key, &claims).expect("assinar"), expected);
        assert_eq!(
            verify(key, expected, 1_900_000_000).expect("verificar"),
            claims
        );
    }

    use super::*;

    const KEY: &[u8] = b"0123456789abcdef0123456789abcdef";

    fn claims(exp: u64) -> Claims {
        Claims {
            sub: "acc_1".into(),
            plan: "pro".into(),
            max_viewers: 50,
            max_rooms: 5,
            max_height: 2160,
            max_fps: 60,
            exp,
        }
    }

    #[test]
    fn roundtrip() {
        let t = sign(KEY, &claims(200)).expect("sign");
        assert_eq!(verify(KEY, &t, 100), Ok(claims(200)));
    }

    #[test]
    fn rejects_expired_tampered_and_other_key() {
        let t = sign(KEY, &claims(200)).expect("sign");
        assert_eq!(verify(KEY, &t, 200), Err(EntitlementError::Expired));
        let other = b"another-key-another-key-another-k";
        assert_eq!(verify(other, &t, 100), Err(EntitlementError::BadSignature));
        let mut forged = claims(200);
        forged.max_viewers = 10_000;
        let forged_body = sign(KEY, &forged).expect("sign");
        let payload = forged_body.split('.').nth(1).unwrap_or_default();
        let sig = t.rsplit('.').next().unwrap_or_default();
        assert_eq!(
            verify(KEY, &format!("ve1.{payload}.{sig}"), 100),
            Err(EntitlementError::BadSignature)
        );
    }

    #[test]
    fn rejects_weak_keys_and_garbage() {
        assert_eq!(sign(b"short", &claims(1)), Err(EntitlementError::WeakKey));
        for t in ["", "ve1", "ve1.x", "v2.a.b", "ve1.@@.##"] {
            assert!(verify(KEY, t, 0).is_err(), "{t}");
        }
        assert_eq!(
            verify(KEY, &"a".repeat(MAX_TOKEN_LEN + 1), 0),
            Err(EntitlementError::Malformed)
        );
    }
}
