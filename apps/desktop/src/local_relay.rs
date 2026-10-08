//! Servidor local automático.
//!
//! Quando o endereço do servidor aponta para este PC (`127.0.0.1`/`localhost`)
//! e ninguém responde, o app inicia o `vellcast-relay` que acompanha o pacote
//! (mesma pasta do executável), sem janela de console, e o encerra ao sair.
//! Assim o teste local não depende de abrir o `iniciar-relay.bat` antes.
//!
//! O segredo do servidor é gerado uma vez e guardado na pasta de configuração
//! do usuário; nunca é registrado em log.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::relay_api;

/// Processo do relay iniciado por este app (encerrado em [`shutdown`]).
static CHILD: Mutex<Option<Child>> = Mutex::new(None);

const START_TIMEOUT: Duration = Duration::from_secs(5);

/// Garante um relay respondendo em `base` quando ele é local. Melhor esforço:
/// se não for local, se já estiver rodando ou se o executável não existir,
/// não faz nada e a chamada seguinte reporta o erro normalmente.
pub fn ensure(base: &str) {
    if !relay_api::is_loopback_base(base) || relay_api::health(base).is_ok() {
        return;
    }
    let Ok(mut guard) = CHILD.lock() else { return };
    // Outro pedido pode ter iniciado o relay enquanto esperávamos o lock.
    if let Some(child) = guard.as_mut() {
        if matches!(child.try_wait(), Ok(None)) {
            wait_healthy(base);
            return;
        }
        *guard = None;
    }
    let Some(exe) = relay_executable() else {
        tracing::info!("servidor local não encontrado ao lado do app; nada a iniciar");
        return;
    };
    let Some(bind) = bind_address(base) else {
        return;
    };
    let secret = match load_or_create_secret() {
        Ok(s) => s,
        Err(err) => {
            tracing::warn!(%err, "não foi possível preparar o segredo do servidor local");
            return;
        }
    };
    let mut cmd = Command::new(&exe);
    cmd.env("VELLCAST_RELAY_SECRET", secret)
        .env("VELLCAST_BIND", &bind)
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(player) = exe
        .parent()
        .map(|d| d.join("player"))
        .filter(|p| p.is_dir())
    {
        cmd.env("VELLCAST_STATIC_DIR", player);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: sem janela de console para o servidor.
        cmd.creation_flags(0x0800_0000);
    }
    match cmd.spawn() {
        Ok(child) => {
            tracing::info!(%bind, "servidor local iniciado pelo app");
            *guard = Some(child);
            drop(guard);
            wait_healthy(base);
        }
        Err(err) => tracing::warn!(%err, exe = %exe.display(), "falha ao iniciar o servidor local"),
    }
}

/// Encerra o relay iniciado por este app, se houver.
pub fn shutdown() {
    if let Ok(mut guard) = CHILD.lock() {
        if let Some(mut child) = guard.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn wait_healthy(base: &str) {
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        if relay_api::health(base).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn relay_executable() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let name = if cfg!(windows) {
        "vellcast-relay.exe"
    } else {
        "vellcast-relay"
    };
    let path = dir.join(name);
    path.is_file().then_some(path)
}

/// Endereço `host:porta` para o relay escutar, derivado do endereço configurado.
pub fn bind_address(base: &str) -> Option<String> {
    let base = relay_api::normalize_base(base).ok()?;
    let (scheme, rest) = base.split_once("://")?;
    let authority = rest.split('/').next()?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h, p.to_string()),
        _ => (
            authority,
            if scheme == "https" { "443" } else { "80" }.to_string(),
        ),
    };
    let host = if host.eq_ignore_ascii_case("localhost") {
        "127.0.0.1"
    } else {
        host
    };
    Some(format!("{host}:{port}"))
}

fn secret_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "Next Estudios", "Vellcast")
        .map(|d| d.config_dir().join("local-relay.secret"))
}

fn load_or_create_secret() -> Result<String, String> {
    let path = secret_path().ok_or("pasta de configuração indisponível")?;
    read_or_create_secret(&path)
}

/// Lê o segredo de `path` ou cria um novo (32 bytes aleatórios em hexadecimal).
pub fn read_or_create_secret(path: &Path) -> Result<String, String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let existing = existing.trim().to_string();
        if existing.len() >= 64 && existing.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(existing);
        }
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
    let secret: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, &secret).map_err(|e| e.to_string())?;
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_bind_address_from_local_base() {
        assert_eq!(
            bind_address("http://127.0.0.1:8788").as_deref(),
            Some("127.0.0.1:8788")
        );
        assert_eq!(
            bind_address("http://localhost:9000/").as_deref(),
            Some("127.0.0.1:9000")
        );
        assert_eq!(
            bind_address("http://localhost").as_deref(),
            Some("127.0.0.1:80")
        );
    }

    #[test]
    fn remote_servers_are_never_started() {
        // Não é loopback: `ensure` retorna sem tentar iniciar nada.
        ensure("https://vellcast.exemplo.invalid");
        assert!(CHILD.lock().map(|c| c.is_none()).unwrap_or(false));
    }

    #[test]
    fn secret_is_created_once_and_reused() {
        let dir = std::env::temp_dir().join(format!("vellcast-secret-{}", std::process::id()));
        let path = dir.join("local-relay.secret");
        let first = read_or_create_secret(&path).expect("criar");
        assert_eq!(first.len(), 64);
        assert_eq!(read_or_create_secret(&path).expect("ler"), first);
        std::fs::write(&path, "curto").expect("sobrescrever");
        let replaced = read_or_create_secret(&path).expect("recriar");
        assert_ne!(replaced, "curto");
        let _ = std::fs::remove_dir_all(dir);
    }
}
