//! Pastas de configuração e de dados do app.
//!
//! Instalado: as pastas do usuário no Windows (`%APPDATA%` e `%LOCALAPPDATA%`,
//! em `Next Estudios\Vellcast`). Portátil (`Vellcast-Portable-x64.exe`, ou
//! qualquer executável com a pasta `Vellcast_Data` ao lado): tudo fica em
//! `Vellcast_Data`, ao lado do executável.

use std::path::{Path, PathBuf};

const PORTABLE_DIR: &str = "Vellcast_Data";

fn is_portable_exe(exe: &Path) -> bool {
    let named = exe
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_ascii_lowercase().contains("portable"));
    named || exe.parent().is_some_and(|d| d.join(PORTABLE_DIR).is_dir())
}

/// Pasta `Vellcast_Data` quando o app roda como portátil.
pub fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    is_portable_exe(&exe).then(|| exe.parent().map(|d| d.join(PORTABLE_DIR)))?
}

pub fn is_portable() -> bool {
    portable_root().is_some()
}

fn project_dirs() -> Option<directories::ProjectDirs> {
    directories::ProjectDirs::from("com", "Next Estudios", "Vellcast")
}

/// Configurações (`settings.json`, segredo do relay local).
pub fn config_dir() -> Option<PathBuf> {
    match portable_root() {
        Some(root) => Some(root.join("config")),
        None => project_dirs().map(|d| d.config_dir().to_path_buf()),
    }
}

/// Dados locais (logs).
pub fn data_dir() -> Option<PathBuf> {
    match portable_root() {
        Some(root) => Some(root.join("data")),
        None => project_dirs().map(|d| d.data_local_dir().to_path_buf()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_by_name_or_data_folder() {
        assert!(is_portable_exe(Path::new("C:/x/Vellcast-Portable-x64.exe")));
        assert!(!is_portable_exe(Path::new(
            "C:/nao-existe/vellcast-desktop.exe"
        )));
        let dir = std::env::temp_dir().join(format!("vellcast-portable-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(PORTABLE_DIR)).expect("pasta temporária");
        assert!(is_portable_exe(&dir.join("vellcast-desktop.exe")));
        let _ = std::fs::remove_dir_all(dir);
    }
}
