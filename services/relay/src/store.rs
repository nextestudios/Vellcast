//! Metadados das salas e convites.
//!
//! [`RoomStore`] é a fronteira de persistência: hoje só existe
//! [`MemoryStore`] (estado perdido ao reiniciar, o que é aceitável para salas
//! temporárias). Uma implementação SQLite para self-hosting ou PostgreSQL
//! para a Cloud implementa o mesmo trait sem tocar no caminho de mídia,
//! que fica sempre em memória (`room::LiveRoom`).

use std::collections::HashMap;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

/// Máximo de convites ativos por sala.
pub const MAX_INVITES_PER_ROOM: usize = 100;

pub type InviteHash = [u8; 32];

/// Os códigos de convite são guardados apenas como SHA-256.
pub fn hash_invite(code: &str) -> InviteHash {
    Sha256::digest(code.as_bytes()).into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub remaining_uses: u32,
    /// Unix, segundos.
    pub expires_at: u64,
}

#[derive(Debug, Clone)]
pub struct RoomRecord {
    pub id: String,
    pub name: String,
    /// Identificador opaco do dono (contas de usuário chegam na ETAPA 5/7).
    pub owner_id: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub max_viewers: usize,
    pub invites: HashMap<InviteHash, Invite>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("sala não encontrada")]
    NotFound,
    #[error("sala já existe")]
    Duplicate,
    #[error("limite de convites da sala atingido")]
    TooManyInvites,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InviteError {
    #[error("convite inválido")]
    NotFound,
    #[error("convite expirado")]
    Expired,
}

pub trait RoomStore: Send + Sync {
    fn insert(&self, record: RoomRecord) -> Result<(), StoreError>;
    fn get(&self, id: &str) -> Option<RoomRecord>;
    fn remove(&self, id: &str) -> Option<RoomRecord>;
    fn add_invite(&self, room: &str, hash: InviteHash, invite: Invite) -> Result<(), StoreError>;
    /// Consome um uso do convite de forma atômica (convites esgotados deixam de existir).
    fn consume_invite(&self, room: &str, hash: &InviteHash, now: u64) -> Result<(), InviteError>;
    /// Remove convites vencidos e devolve as salas cujo TTL acabou.
    fn sweep(&self, now: u64) -> Vec<String>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Default)]
pub struct MemoryStore {
    rooms: Mutex<HashMap<String, RoomRecord>>,
}

impl MemoryStore {
    fn rooms(&self) -> std::sync::MutexGuard<'_, HashMap<String, RoomRecord>> {
        // Um panic com o lock tomado não deixa o mapa inconsistente (operações curtas).
        self.rooms.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl RoomStore for MemoryStore {
    fn insert(&self, record: RoomRecord) -> Result<(), StoreError> {
        let mut rooms = self.rooms();
        if rooms.contains_key(&record.id) {
            return Err(StoreError::Duplicate);
        }
        rooms.insert(record.id.clone(), record);
        Ok(())
    }

    fn get(&self, id: &str) -> Option<RoomRecord> {
        self.rooms().get(id).cloned()
    }

    fn remove(&self, id: &str) -> Option<RoomRecord> {
        self.rooms().remove(id)
    }

    fn add_invite(&self, room: &str, hash: InviteHash, invite: Invite) -> Result<(), StoreError> {
        let mut rooms = self.rooms();
        let r = rooms.get_mut(room).ok_or(StoreError::NotFound)?;
        if r.invites.len() >= MAX_INVITES_PER_ROOM {
            return Err(StoreError::TooManyInvites);
        }
        r.invites.insert(hash, invite);
        Ok(())
    }

    fn consume_invite(&self, room: &str, hash: &InviteHash, now: u64) -> Result<(), InviteError> {
        let mut rooms = self.rooms();
        let r = rooms.get_mut(room).ok_or(InviteError::NotFound)?;
        let invite = r.invites.get_mut(hash).ok_or(InviteError::NotFound)?;
        if invite.expires_at <= now {
            r.invites.remove(hash);
            return Err(InviteError::Expired);
        }
        invite.remaining_uses = invite.remaining_uses.saturating_sub(1);
        if invite.remaining_uses == 0 {
            r.invites.remove(hash);
        }
        Ok(())
    }

    fn sweep(&self, now: u64) -> Vec<String> {
        let mut rooms = self.rooms();
        let mut expired = Vec::new();
        for (id, r) in rooms.iter_mut() {
            r.invites.retain(|_, i| i.expires_at > now);
            if r.expires_at <= now {
                expired.push(id.clone());
            }
        }
        expired
    }

    fn len(&self) -> usize {
        self.rooms().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> RoomRecord {
        RoomRecord {
            id: id.into(),
            name: "sala".into(),
            owner_id: "dono".into(),
            created_at: 0,
            expires_at: 1000,
            max_viewers: 10,
            invites: HashMap::new(),
        }
    }

    #[test]
    fn invite_max_uses() {
        let s = MemoryStore::default();
        s.insert(record("r")).expect("insert");
        let h = hash_invite("codigo");
        s.add_invite(
            "r",
            h,
            Invite {
                remaining_uses: 2,
                expires_at: 100,
            },
        )
        .expect("invite");
        assert_eq!(s.consume_invite("r", &h, 10), Ok(()));
        assert_eq!(s.consume_invite("r", &h, 10), Ok(()));
        assert_eq!(s.consume_invite("r", &h, 10), Err(InviteError::NotFound));
    }

    #[test]
    fn invite_expiry_and_wrong_code() {
        let s = MemoryStore::default();
        s.insert(record("r")).expect("insert");
        let h = hash_invite("codigo");
        let inv = Invite {
            remaining_uses: 5,
            expires_at: 100,
        };
        s.add_invite("r", h, inv).expect("invite");
        assert_eq!(
            s.consume_invite("r", &hash_invite("outro"), 10),
            Err(InviteError::NotFound)
        );
        assert_eq!(
            s.consume_invite("other", &h, 10),
            Err(InviteError::NotFound)
        );
        assert_eq!(s.consume_invite("r", &h, 100), Err(InviteError::Expired));
    }

    #[test]
    fn sweep_returns_expired_rooms_and_purges_invites() {
        let s = MemoryStore::default();
        s.insert(record("a")).expect("insert");
        let mut b = record("b");
        b.expires_at = 5000;
        s.insert(b).expect("insert");
        assert_eq!(s.insert(record("a")), Err(StoreError::Duplicate));
        s.add_invite(
            "b",
            hash_invite("x"),
            Invite {
                remaining_uses: 1,
                expires_at: 1500,
            },
        )
        .expect("invite");
        assert_eq!(s.sweep(2000), vec!["a".to_string()]);
        assert!(s.get("b").is_some_and(|r| r.invites.is_empty()));
    }

    #[test]
    fn invite_limit_per_room() {
        let s = MemoryStore::default();
        s.insert(record("r")).expect("insert");
        for i in 0..MAX_INVITES_PER_ROOM {
            let inv = Invite {
                remaining_uses: 1,
                expires_at: 100,
            };
            s.add_invite("r", hash_invite(&i.to_string()), inv)
                .expect("invite");
        }
        let inv = Invite {
            remaining_uses: 1,
            expires_at: 100,
        };
        assert_eq!(
            s.add_invite("r", hash_invite("extra"), inv),
            Err(StoreError::TooManyInvites)
        );
    }
}
