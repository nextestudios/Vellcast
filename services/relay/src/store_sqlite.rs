//! [`RoomStore`] persistente em SQLite (`VELLCAST_DB_PATH`).
//!
//! Guarda apenas metadados: salas, convites (como hash SHA-256) e revogações.
//! Mídia nunca é gravada. O banco usa WAL e as migrações rodam na abertura,
//! controladas por `PRAGMA user_version`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};

use crate::store::{
    Invite, InviteError, InviteHash, MAX_INVITES_PER_ROOM, RoomRecord, RoomStore, StoreError,
};

/// Migrações em ordem; o índice + 1 é a versão do esquema.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE rooms (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        owner_id TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        max_viewers INTEGER NOT NULL,
        viewers_revoked_before INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE invites (
        room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
        hash BLOB NOT NULL,
        remaining_uses INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        PRIMARY KEY (room_id, hash)
    );
    CREATE TABLE revoked_tokens (
        room_id TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
        nonce TEXT NOT NULL,
        expires_at INTEGER NOT NULL,
        PRIMARY KEY (room_id, nonce)
    );
"];

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

fn storage(e: rusqlite::Error) -> StoreError {
    StoreError::Storage(e.to_string())
}

impl SqliteStore {
    /// Abre (ou cria) o banco e aplica as migrações pendentes.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Storage(e.to_string()))?;
        }
        Self::init(Connection::open(path).map_err(storage)?)
    }

    /// Banco em memória (testes).
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory().map_err(storage)?)
    }

    fn init(conn: Connection) -> Result<Self, StoreError> {
        // `journal_mode` devolve o modo efetivo ("memory" em bancos em memória).
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0))
            .map_err(storage)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(storage)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(storage)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(storage)?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(storage)?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version.max(0) as usize) {
            conn.execute_batch(sql).map_err(storage)?;
            conn.pragma_update(None, "user_version", (i + 1) as i64)
                .map_err(storage)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // Operações curtas; um panic com o lock tomado não deixa o banco inconsistente.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn load(conn: &Connection, id: &str) -> rusqlite::Result<Option<RoomRecord>> {
        let Some(mut record) = conn
            .query_row(
                "SELECT id, name, owner_id, created_at, expires_at, max_viewers, viewers_revoked_before
                 FROM rooms WHERE id = ?1",
                params![id],
                |r| {
                    Ok(RoomRecord {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        owner_id: r.get(2)?,
                        created_at: r.get::<_, i64>(3)? as u64,
                        expires_at: r.get::<_, i64>(4)? as u64,
                        max_viewers: r.get::<_, i64>(5)? as usize,
                        invites: HashMap::new(),
                        revoked: HashMap::new(),
                        viewers_revoked_before: r.get::<_, i64>(6)? as u64,
                    })
                },
            )
            .optional()?
        else {
            return Ok(None);
        };
        let mut stmt = conn.prepare_cached(
            "SELECT hash, remaining_uses, expires_at FROM invites WHERE room_id = ?1",
        )?;
        let invites = stmt.query_map(params![id], |r| {
            let hash: Vec<u8> = r.get(0)?;
            Ok((
                hash,
                Invite {
                    remaining_uses: r.get::<_, i64>(1)? as u32,
                    expires_at: r.get::<_, i64>(2)? as u64,
                },
            ))
        })?;
        for row in invites {
            let (hash, invite) = row?;
            if let Ok(hash) = InviteHash::try_from(hash.as_slice()) {
                record.invites.insert(hash, invite);
            }
        }
        let mut stmt =
            conn.prepare_cached("SELECT nonce, expires_at FROM revoked_tokens WHERE room_id = ?1")?;
        let revoked = stmt.query_map(params![id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
        })?;
        for row in revoked {
            let (nonce, exp) = row?;
            record.revoked.insert(nonce, exp);
        }
        Ok(Some(record))
    }
}

impl RoomStore for SqliteStore {
    fn insert(&self, record: RoomRecord) -> Result<(), StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(storage)?;
        let inserted = tx
            .execute(
                "INSERT OR IGNORE INTO rooms
                 (id, name, owner_id, created_at, expires_at, max_viewers, viewers_revoked_before)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    record.id,
                    record.name,
                    record.owner_id,
                    record.created_at as i64,
                    record.expires_at as i64,
                    record.max_viewers as i64,
                    record.viewers_revoked_before as i64,
                ],
            )
            .map_err(storage)?;
        if inserted == 0 {
            return Err(StoreError::Duplicate);
        }
        for (hash, invite) in &record.invites {
            tx.execute(
                "INSERT INTO invites (room_id, hash, remaining_uses, expires_at) VALUES (?1, ?2, ?3, ?4)",
                params![record.id, hash.as_slice(), invite.remaining_uses as i64, invite.expires_at as i64],
            )
            .map_err(storage)?;
        }
        for (nonce, exp) in &record.revoked {
            tx.execute(
                "INSERT INTO revoked_tokens (room_id, nonce, expires_at) VALUES (?1, ?2, ?3)",
                params![record.id, nonce, *exp as i64],
            )
            .map_err(storage)?;
        }
        tx.commit().map_err(storage)
    }

    fn get(&self, id: &str) -> Option<RoomRecord> {
        match Self::load(&self.conn(), id) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(error = %e, "falha ao ler sala do banco");
                None
            }
        }
    }

    fn remove(&self, id: &str) -> Option<RoomRecord> {
        let conn = self.conn();
        let record = Self::load(&conn, id).ok().flatten()?;
        if let Err(e) = conn.execute("DELETE FROM rooms WHERE id = ?1", params![id]) {
            tracing::error!(error = %e, "falha ao remover sala do banco");
        }
        Some(record)
    }

    fn add_invite(&self, room: &str, hash: InviteHash, invite: Invite) -> Result<(), StoreError> {
        let mut conn = self.conn();
        let tx = conn.transaction().map_err(storage)?;
        let exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?1)",
                params![room],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        let count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM invites WHERE room_id = ?1",
                params![room],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if count as usize >= MAX_INVITES_PER_ROOM {
            return Err(StoreError::TooManyInvites);
        }
        tx.execute(
            "INSERT OR REPLACE INTO invites (room_id, hash, remaining_uses, expires_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                room,
                hash.as_slice(),
                invite.remaining_uses as i64,
                invite.expires_at as i64
            ],
        )
        .map_err(storage)?;
        tx.commit().map_err(storage)
    }

    fn consume_invite(&self, room: &str, hash: &InviteHash, now: u64) -> Result<(), InviteError> {
        let mut conn = self.conn();
        // Transação: leitura e decremento atômicos (sem dois usos do último convite).
        let tx = conn.transaction().map_err(|_| InviteError::NotFound)?;
        let row: Option<(i64, i64)> = tx
            .query_row(
                "SELECT remaining_uses, expires_at FROM invites WHERE room_id = ?1 AND hash = ?2",
                params![room, hash.as_slice()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(|_| InviteError::NotFound)?;
        let Some((remaining, expires_at)) = row else {
            return Err(InviteError::NotFound);
        };
        let result = if (expires_at as u64) <= now {
            Err(InviteError::Expired)
        } else {
            Ok(())
        };
        if result.is_err() || remaining <= 1 {
            tx.execute(
                "DELETE FROM invites WHERE room_id = ?1 AND hash = ?2",
                params![room, hash.as_slice()],
            )
            .map_err(|_| InviteError::NotFound)?;
        } else {
            tx.execute(
                "UPDATE invites SET remaining_uses = remaining_uses - 1 WHERE room_id = ?1 AND hash = ?2",
                params![room, hash.as_slice()],
            )
            .map_err(|_| InviteError::NotFound)?;
        }
        tx.commit().map_err(|_| InviteError::NotFound)?;
        result
    }

    fn revoke_token(&self, room: &str, nonce: &str, token_exp: u64) -> Result<(), StoreError> {
        let conn = self.conn();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM rooms WHERE id = ?1)",
                params![room],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        conn.execute(
            "INSERT OR REPLACE INTO revoked_tokens (room_id, nonce, expires_at) VALUES (?1, ?2, ?3)",
            params![room, nonce, token_exp as i64],
        )
        .map_err(storage)?;
        Ok(())
    }

    fn revoke_viewers_before(&self, room: &str, before: u64) -> Result<(), StoreError> {
        let changed = self
            .conn()
            .execute(
                "UPDATE rooms SET viewers_revoked_before = MAX(viewers_revoked_before, ?2) WHERE id = ?1",
                params![room, before as i64],
            )
            .map_err(storage)?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    fn list(&self) -> Vec<RoomRecord> {
        let conn = self.conn();
        let ids: Vec<String> = match conn.prepare("SELECT id FROM rooms").and_then(|mut s| {
            s.query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()
        }) {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!(error = %e, "falha ao listar salas do banco");
                return Vec::new();
            }
        };
        ids.iter()
            .filter_map(|id| Self::load(&conn, id).ok().flatten())
            .collect()
    }

    fn sweep(&self, now: u64) -> Vec<String> {
        let conn = self.conn();
        let now = now as i64;
        let cleanup = conn
            .execute("DELETE FROM invites WHERE expires_at <= ?1", params![now])
            .and_then(|_| {
                conn.execute(
                    "DELETE FROM revoked_tokens WHERE expires_at <= ?1",
                    params![now],
                )
            });
        if let Err(e) = cleanup {
            tracing::error!(error = %e, "falha na limpeza do banco");
        }
        conn.prepare("SELECT id FROM rooms WHERE expires_at <= ?1")
            .and_then(|mut s| {
                s.query_map(params![now], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .unwrap_or_default()
    }

    fn len(&self) -> usize {
        self.conn()
            .query_row("SELECT COUNT(*) FROM rooms", [], |r| r.get::<_, i64>(0))
            .map_or(0, |n| n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::hash_invite;

    fn record(id: &str) -> RoomRecord {
        RoomRecord {
            id: id.into(),
            name: "sala".into(),
            owner_id: "dono".into(),
            created_at: 0,
            expires_at: 1000,
            max_viewers: 10,
            invites: HashMap::new(),
            revoked: HashMap::new(),
            viewers_revoked_before: 0,
        }
    }

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vellcast-relay-db-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("relay.db")
    }

    #[test]
    fn rooms_invites_and_revocations_survive_reopen() {
        let path = temp_db("reopen");
        {
            let s = SqliteStore::open(&path).expect("abrir");
            s.insert(record("r")).expect("inserir");
            s.add_invite(
                "r",
                hash_invite("c"),
                Invite {
                    remaining_uses: 3,
                    expires_at: 500,
                },
            )
            .expect("convite");
            assert_eq!(s.consume_invite("r", &hash_invite("c"), 10), Ok(()));
            s.revoke_token("r", "nonce-x", 900).expect("revogar");
            s.revoke_viewers_before("r", 50)
                .expect("revogar espectadores");
        }
        let s = SqliteStore::open(&path).expect("reabrir");
        let r = s.get("r").expect("sala persistida");
        assert_eq!(r.name, "sala");
        assert_eq!(
            r.invites[&hash_invite("c")].remaining_uses,
            2,
            "uso consumido persistido"
        );
        assert!(r.is_revoked("nonce-x", false, 999));
        assert!(r.is_revoked("outro", true, 49));
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.len(), 1);
        let _ = std::fs::remove_dir_all(path.parent().unwrap_or(&path));
    }

    #[test]
    fn invite_semantics_match_memory_store() {
        let s = SqliteStore::open_in_memory().expect("abrir");
        s.insert(record("r")).expect("inserir");
        assert_eq!(s.insert(record("r")), Err(StoreError::Duplicate));
        let h = hash_invite("codigo");
        s.add_invite(
            "r",
            h,
            Invite {
                remaining_uses: 2,
                expires_at: 100,
            },
        )
        .expect("convite");
        assert_eq!(s.consume_invite("r", &h, 10), Ok(()));
        assert_eq!(s.consume_invite("r", &h, 10), Ok(()));
        assert_eq!(s.consume_invite("r", &h, 10), Err(InviteError::NotFound));
        s.add_invite(
            "r",
            h,
            Invite {
                remaining_uses: 5,
                expires_at: 100,
            },
        )
        .expect("convite");
        assert_eq!(s.consume_invite("r", &h, 100), Err(InviteError::Expired));
        assert_eq!(
            s.add_invite(
                "nao-existe",
                h,
                Invite {
                    remaining_uses: 1,
                    expires_at: 100
                }
            ),
            Err(StoreError::NotFound)
        );
    }

    #[test]
    fn sweep_and_remove() {
        let s = SqliteStore::open_in_memory().expect("abrir");
        s.insert(record("a")).expect("a");
        let mut b = record("b");
        b.expires_at = 5000;
        s.insert(b).expect("b");
        s.revoke_token("b", "n", 1500).expect("revogar");
        assert_eq!(s.sweep(2000), vec!["a".to_string()]);
        assert!(s.get("b").expect("b").revoked.is_empty());
        assert!(s.remove("a").is_some());
        assert!(s.get("a").is_none());
        assert_eq!(s.len(), 1);
    }
}
