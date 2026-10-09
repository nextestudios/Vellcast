//! Pareamento entre o app desktop e uma instância da Discord Activity (M-015).
//!
//! O dono da sala pede um código curto (8 caracteres, 5 min, uso único) e o
//! mostra no desktop. Quem está na Activity digita o código; o relay então
//! vincula a instância da Activity à sala, e qualquer participante dessa
//! instância passa a ver a transmissão. Estado só em memória, como as salas.

use std::collections::HashMap;
use std::sync::Mutex;

/// Alfabeto sem caracteres ambíguos (0/O, 1/I/L).
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 8;
pub const CODE_TTL_SECS: u64 = 5 * 60;
/// Códigos ativos por sala (evita acúmulo por cliques repetidos).
pub const MAX_CODES_PER_ROOM: usize = 3;
/// Salas vinculadas a uma mesma instância da Activity.
pub const MAX_ROOMS_PER_INSTANCE: usize = 8;
/// Tamanho máximo aceito para `instance_id` vindo do cliente.
pub const MAX_INSTANCE_ID: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    room: String,
    expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub room: String,
    /// Expira junto com a sala.
    pub expires_at: u64,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PairError {
    #[error("código inválido ou expirado")]
    InvalidCode,
    #[error("limite de códigos ativos para a sala")]
    TooManyCodes,
    #[error("limite de salas vinculadas a esta instância")]
    TooManyRooms,
    #[error("instância inválida")]
    InvalidInstance,
}

#[derive(Default)]
struct State {
    codes: HashMap<String, Pending>,
    instances: HashMap<String, Vec<Binding>>,
}

#[derive(Default)]
pub struct PairingStore {
    state: Mutex<State>,
}

/// Normaliza o código digitado: maiúsculas, sem espaços nem hífens.
pub fn normalize_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// `instance_id` aceitável: não vazio, curto e só com caracteres seguros.
pub fn valid_instance_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_INSTANCE_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
}

fn random_code() -> String {
    let bytes = crate::random_bytes::<CODE_LEN>();
    bytes
        .iter()
        .map(|b| ALPHABET[usize::from(*b) % ALPHABET.len()] as char)
        .collect()
}

impl PairingStore {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Cria um código de uso único para `room`.
    pub fn create_code(
        &self,
        room: &str,
        now: u64,
        room_expires_at: u64,
    ) -> Result<(String, u64), PairError> {
        let mut s = self.state();
        s.codes.retain(|_, p| p.expires_at > now);
        if s.codes.values().filter(|p| p.room == room).count() >= MAX_CODES_PER_ROOM {
            return Err(PairError::TooManyCodes);
        }
        let expires_at = (now + CODE_TTL_SECS).min(room_expires_at);
        let code = loop {
            let c = random_code();
            if !s.codes.contains_key(&c) {
                break c;
            }
        };
        s.codes.insert(
            code.clone(),
            Pending {
                room: room.to_owned(),
                expires_at,
            },
        );
        Ok((code, expires_at))
    }

    /// Consome o código e vincula a instância à sala. Retorna a sala.
    pub fn pair(
        &self,
        code: &str,
        instance: &str,
        now: u64,
        room_expires_at: impl Fn(&str) -> Option<u64>,
    ) -> Result<String, PairError> {
        if !valid_instance_id(instance) {
            return Err(PairError::InvalidInstance);
        }
        let code = normalize_code(code);
        let mut s = self.state();
        let pending = match s.codes.get(&code) {
            Some(p) if p.expires_at > now => p.clone(),
            _ => return Err(PairError::InvalidCode),
        };
        let Some(room_expiry) = room_expires_at(&pending.room) else {
            s.codes.remove(&code);
            return Err(PairError::InvalidCode);
        };
        let bindings = s.instances.entry(instance.to_owned()).or_default();
        bindings.retain(|b| b.expires_at > now);
        if !bindings.iter().any(|b| b.room == pending.room) {
            if bindings.len() >= MAX_ROOMS_PER_INSTANCE {
                return Err(PairError::TooManyRooms);
            }
            bindings.push(Binding {
                room: pending.room.clone(),
                expires_at: room_expiry,
            });
        }
        // Uso único: removido só depois de vincular com sucesso.
        s.codes.remove(&code);
        Ok(pending.room)
    }

    /// Salas vinculadas à instância e ainda válidas.
    pub fn rooms_for(&self, instance: &str, now: u64) -> Vec<String> {
        let mut s = self.state();
        match s.instances.get_mut(instance) {
            Some(bindings) => {
                bindings.retain(|b| b.expires_at > now);
                bindings.iter().map(|b| b.room.clone()).collect()
            }
            None => Vec::new(),
        }
    }

    /// Remove códigos e vínculos vencidos.
    pub fn sweep(&self, now: u64) {
        let mut s = self.state();
        s.codes.retain(|_, p| p.expires_at > now);
        s.instances.retain(|_, b| {
            b.retain(|x| x.expires_at > now);
            !b.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000;

    fn alive(_: &str) -> Option<u64> {
        Some(NOW + 3600)
    }

    #[test]
    fn codes_are_short_unambiguous_and_single_use() {
        let store = PairingStore::default();
        let (code, exp) = store.create_code("room1", NOW, NOW + 3600).expect("código");
        assert_eq!(code.len(), CODE_LEN);
        assert!(code.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(exp, NOW + CODE_TTL_SECS);
        assert_eq!(store.pair(&code, "inst-1", NOW, alive), Ok("room1".into()));
        assert_eq!(
            store.pair(&code, "inst-2", NOW, alive),
            Err(PairError::InvalidCode)
        );
        assert_eq!(store.rooms_for("inst-1", NOW), vec!["room1".to_string()]);
        assert!(store.rooms_for("inst-2", NOW).is_empty());
    }

    #[test]
    fn code_expires() {
        let store = PairingStore::default();
        let (code, _) = store.create_code("room1", NOW, NOW + 3600).expect("código");
        assert_eq!(
            store.pair(&code, "inst", NOW + CODE_TTL_SECS, alive),
            Err(PairError::InvalidCode)
        );
    }

    #[test]
    fn code_never_outlives_room() {
        let store = PairingStore::default();
        let (_, exp) = store.create_code("room1", NOW, NOW + 30).expect("código");
        assert_eq!(exp, NOW + 30);
    }

    #[test]
    fn wrong_code_and_normalization() {
        let store = PairingStore::default();
        let (code, _) = store.create_code("room1", NOW, NOW + 3600).expect("código");
        assert_eq!(
            store.pair("ZZZZZZZZ", "inst", NOW, alive),
            Err(PairError::InvalidCode)
        );
        let typed = format!(" {}-{} ", code[..4].to_lowercase(), &code[4..]);
        assert_eq!(store.pair(&typed, "inst", NOW, alive), Ok("room1".into()));
    }

    #[test]
    fn closed_room_invalidates_code() {
        let store = PairingStore::default();
        let (code, _) = store.create_code("room1", NOW, NOW + 3600).expect("código");
        assert_eq!(
            store.pair(&code, "inst", NOW, |_| None),
            Err(PairError::InvalidCode)
        );
    }

    #[test]
    fn limits_codes_per_room_and_rooms_per_instance() {
        let store = PairingStore::default();
        for _ in 0..MAX_CODES_PER_ROOM {
            store.create_code("room1", NOW, NOW + 3600).expect("código");
        }
        assert_eq!(
            store.create_code("room1", NOW, NOW + 3600),
            Err(PairError::TooManyCodes)
        );
        for i in 0..MAX_ROOMS_PER_INSTANCE {
            let (c, _) = store
                .create_code(&format!("r{i}"), NOW, NOW + 3600)
                .expect("código");
            store.pair(&c, "inst", NOW, alive).expect("vínculo");
        }
        let (c, _) = store.create_code("extra", NOW, NOW + 3600).expect("código");
        assert_eq!(
            store.pair(&c, "inst", NOW, alive),
            Err(PairError::TooManyRooms)
        );
    }

    #[test]
    fn bindings_expire_with_room_and_sweep() {
        let store = PairingStore::default();
        let (c, _) = store.create_code("room1", NOW, NOW + 100).expect("código");
        store
            .pair(&c, "inst", NOW, |_| Some(NOW + 100))
            .expect("vínculo");
        assert_eq!(store.rooms_for("inst", NOW + 99).len(), 1);
        assert!(store.rooms_for("inst", NOW + 100).is_empty());
        store.sweep(NOW + 200);
        assert!(store.rooms_for("inst", NOW).is_empty());
    }

    #[test]
    fn rejects_unsafe_instance_ids() {
        assert!(valid_instance_id("i-1234567890:abc"));
        assert!(!valid_instance_id(""));
        assert!(!valid_instance_id("a/b"));
        assert!(!valid_instance_id(&"a".repeat(MAX_INSTANCE_ID + 1)));
    }
}
