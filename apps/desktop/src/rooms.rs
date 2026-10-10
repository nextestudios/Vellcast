//! Salas criadas por este computador, persistidas em `rooms.json` ao lado
//! das configurações.
//!
//! O arquivo guarda o token de dono de cada sala (necessário para gerar
//! convites, tokens de transmissor e encerrar a sala). Ele fica no diretório
//! de configuração do usuário e nunca é registrado em log.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Limite de salas guardadas; as mais antigas saem primeiro.
pub const MAX_SAVED_ROOMS: usize = 20;

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRoom {
    pub id: String,
    pub name: String,
    /// Endereço base do relay em que a sala foi criada.
    pub relay: String,
    /// Vazio nas salas em que este PC entrou por convite.
    pub owner_token: String,
    pub created_at: u64,
    pub expires_at: u64,
    /// Sala de outra pessoa, em que este PC entrou com um link de convite
    /// (participante: assiste e pode compartilhar, mas não gerencia a sala).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub joined: bool,
    /// Token de participante (só nas salas em que entrou por convite).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub viewer_token: String,
}

impl SavedRoom {
    /// Token usado para consultar a sala e pedir para compartilhar.
    pub fn access_token(&self) -> &str {
        if self.joined {
            &self.viewer_token
        } else {
            &self.owner_token
        }
    }
}

// Debug manual: o token de dono nunca aparece em logs.
impl std::fmt::Debug for SavedRoom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedRoom")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("relay", &self.relay)
            .field("owner_token", &"<oculto>")
            .field("viewer_token", &"<oculto>")
            .field("joined", &self.joined)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomBook {
    pub rooms: Vec<SavedRoom>,
}

impl RoomBook {
    /// Remove salas expiradas.
    pub fn prune(&mut self, now: u64) {
        self.rooms.retain(|r| r.expires_at > now);
    }

    /// Adiciona no topo da lista, respeitando o limite.
    pub fn add(&mut self, room: SavedRoom) {
        self.rooms
            .retain(|r| r.id != room.id || r.relay != room.relay);
        self.rooms.insert(0, room);
        self.rooms.truncate(MAX_SAVED_ROOMS);
    }

    pub fn remove(&mut self, id: &str) {
        self.rooms.retain(|r| r.id != id);
    }

    pub fn get(&self, id: &str) -> Option<&SavedRoom> {
        self.rooms.iter().find(|r| r.id == id)
    }
}

pub fn path_next_to(settings_path: &Path) -> PathBuf {
    settings_path.with_file_name("rooms.json")
}

/// Carrega o arquivo; ausente ou corrompido resulta em lista vazia
/// (o corrompido é preservado como `.corrupt`).
pub fn load(path: &Path, now: u64) -> RoomBook {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return RoomBook::default(),
        Err(e) => {
            tracing::warn!(path = %path.display(), %e, "não foi possível ler as salas");
            return RoomBook::default();
        }
    };
    match serde_json::from_slice::<RoomBook>(&raw) {
        Ok(mut book) => {
            book.prune(now);
            book.rooms.truncate(MAX_SAVED_ROOMS);
            book
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), %e, "arquivo de salas corrompido; começando vazio");
            let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
            RoomBook::default()
        }
    }
}

/// Grava de forma atômica (temporário + rename).
pub fn save(path: &Path, book: &RoomBook) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(book).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn room(id: &str, expires_at: u64) -> SavedRoom {
        SavedRoom {
            id: id.into(),
            name: format!("Sala {id}"),
            relay: "http://127.0.0.1:8788".into(),
            owner_token: "segredo-do-dono".into(),
            created_at: 1,
            expires_at,
            ..SavedRoom::default()
        }
    }

    #[test]
    fn joined_rooms_roundtrip_and_old_files_still_load() {
        let dir = temp_dir("joined");
        let path = dir.join("rooms.json");
        let mut book = RoomBook::default();
        book.add(SavedRoom {
            joined: true,
            viewer_token: "token-do-participante".into(),
            owner_token: String::new(),
            ..room("p", 300)
        });
        book.add(room("d", 300));
        save(&path, &book).expect("save");
        let loaded = load(&path, 0);
        assert_eq!(loaded, book);
        let joined = loaded.get("p").expect("sala");
        assert_eq!(joined.access_token(), "token-do-participante");
        assert_eq!(
            loaded.get("d").expect("sala").access_token(),
            "segredo-do-dono"
        );
        assert!(!format!("{joined:?}").contains("token-do-participante"));
        // Arquivo de versões anteriores (sem os campos novos) continua válido.
        let old = br#"{"rooms":[{"id":"a","name":"A","relay":"http://x","owner_token":"t","created_at":1,"expires_at":300}]}"#;
        std::fs::write(&path, old).expect("write");
        let loaded = load(&path, 0);
        assert!(!loaded.rooms[0].joined);
        assert_eq!(loaded.rooms[0].access_token(), "t");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vellcast-rooms-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn roundtrip_and_prune_expired() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("rooms.json");
        let mut book = RoomBook::default();
        book.add(room("a", 100));
        book.add(room("b", 300));
        save(&path, &book).expect("save");
        let loaded = load(&path, 200);
        assert_eq!(loaded.rooms.len(), 1);
        assert_eq!(loaded.rooms[0].id, "b");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_is_preserved() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("rooms.json");
        std::fs::write(&path, b"[nao e json").expect("write");
        assert_eq!(load(&path, 0), RoomBook::default());
        assert!(dir.join("rooms.json.corrupt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_file_is_empty() {
        assert_eq!(
            load(&temp_dir("missing").join("rooms.json"), 0),
            RoomBook::default()
        );
    }

    #[test]
    fn add_dedupes_and_caps() {
        let mut book = RoomBook::default();
        for i in 0..(MAX_SAVED_ROOMS + 5) {
            book.add(room(&i.to_string(), 10));
        }
        book.add(room("3", 10));
        assert_eq!(book.rooms.len(), MAX_SAVED_ROOMS);
        assert_eq!(book.rooms[0].id, "3");
        assert_eq!(book.rooms.iter().filter(|r| r.id == "3").count(), 1);
        book.remove("3");
        assert!(book.get("3").is_none());
    }

    #[test]
    fn debug_never_prints_owner_token() {
        let text = format!("{:?}", room("x", 1));
        assert!(!text.contains("segredo-do-dono"));
    }
}
