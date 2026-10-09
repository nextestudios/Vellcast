//! Limites de plano no relay hospedado (E7-T04).
//!
//! Só se aplica quando `VELLCAST_CLOUD_ENTITLEMENT_KEY` está definido (serviço
//! oficial). Nesse caso, `POST /api/rooms` exige o cabeçalho
//! `X-Vellcast-Entitlement` com um token emitido pela Vellcast Cloud, e o relay
//! aplica o máximo de espectadores por sala e de salas por conta do plano.
//! Sem a chave (self-hosting), nada muda e nenhum token é pedido (W-013).

use std::collections::HashMap;
use std::sync::Mutex;

use axum::http::HeaderMap;
use vellcast_entitlement::Claims;

use crate::{Relay, unix_now};

pub const HEADER: &str = "x-vellcast-entitlement";

#[derive(Debug, PartialEq, Eq)]
pub enum EntitlementDenied {
    /// Token ausente, inválido ou expirado.
    Invalid,
    /// A conta já tem o máximo de salas do plano.
    RoomLimit,
}

/// Salas criadas por conta (só no modo hospedado).
#[derive(Default)]
pub struct AccountRooms {
    rooms: Mutex<HashMap<String, Vec<String>>>,
}

impl AccountRooms {
    /// Registra a sala criada pela conta.
    pub fn record(&self, account: &str, room: &str) {
        let mut map = self.rooms.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(account.to_string())
            .or_default()
            .push(room.to_string());
    }

    /// Salas ainda existentes da conta (as encerradas ou expiradas são esquecidas).
    fn live_count(&self, relay: &Relay, account: &str) -> usize {
        let mut map = self.rooms.lock().unwrap_or_else(|p| p.into_inner());
        let Some(list) = map.get_mut(account) else {
            return 0;
        };
        list.retain(|id| relay.store.get(id).is_some());
        let n = list.len();
        if n == 0 {
            map.remove(account);
        }
        n
    }
}

/// Valida o entitlement do pedido de criação de sala.
///
/// `Ok(None)` quando o relay não está no modo hospedado.
pub fn authorize(relay: &Relay, headers: &HeaderMap) -> Result<Option<Claims>, EntitlementDenied> {
    let Some(key) = relay.config.cloud_entitlement_key.as_deref() else {
        return Ok(None);
    };
    let token = headers
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or(EntitlementDenied::Invalid)?;
    let claims = vellcast_entitlement::verify(key, token, unix_now())
        .map_err(|_| EntitlementDenied::Invalid)?;
    if relay.account_rooms.live_count(relay, &claims.sub) >= claims.max_rooms as usize {
        return Err(EntitlementDenied::RoomLimit);
    }
    Ok(Some(claims))
}
