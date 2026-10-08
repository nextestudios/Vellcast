//! Configurações persistidas em JSON no diretório de configuração do sistema
//! (Windows: `%APPDATA%\Next Estudios\Vellcast\config\settings.json`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use vellcast_controller::InputConfig;

pub const MAX_DISPLAY_NAME_CHARS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Language {
    #[default]
    #[serde(rename = "pt-BR")]
    PtBr,
    #[serde(rename = "en")]
    En,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::PtBr => "pt-BR",
            Language::En => "en",
        }
    }

    pub fn from_index(i: i32) -> Self {
        if i == 1 { Language::En } else { Language::PtBr }
    }

    pub fn index(self) -> i32 {
        match self {
            Language::PtBr => 0,
            Language::En => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    /// Nome mostrado aos espectadores (usado a partir da Etapa 5).
    pub display_name: String,
    pub gamepad_deadzone: f32,
    pub gamepad_repeat_delay_ms: u64,
    pub gamepad_repeat_interval_ms: u64,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    /// Atalhos globais em segundo plano. Desativados por padrão (spec §13);
    /// nenhum atalho global existe ainda.
    pub background_shortcuts: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let input = InputConfig::default();
        Self {
            language: Language::default(),
            display_name: String::new(),
            gamepad_deadzone: input.deadzone,
            gamepad_repeat_delay_ms: input.repeat_delay.as_millis() as u64,
            gamepad_repeat_interval_ms: input.repeat_interval.as_millis() as u64,
            start_minimized: false,
            close_to_tray: true,
            background_shortcuts: false,
        }
    }
}

impl Settings {
    /// Corrige valores fora de faixa vindos de um arquivo editado à mão.
    pub fn sanitized(mut self) -> Self {
        let input = self.input_config();
        self.gamepad_deadzone = input.deadzone;
        self.gamepad_repeat_delay_ms = input.repeat_delay.as_millis() as u64;
        self.gamepad_repeat_interval_ms = input.repeat_interval.as_millis() as u64;
        self.display_name = sanitize_display_name(&self.display_name);
        self
    }

    pub fn input_config(&self) -> InputConfig {
        InputConfig {
            deadzone: self.gamepad_deadzone,
            repeat_delay: Duration::from_millis(self.gamepad_repeat_delay_ms),
            repeat_interval: Duration::from_millis(self.gamepad_repeat_interval_ms),
        }
        .sanitized()
    }
}

/// Remove caracteres de controle e limita o tamanho.
pub fn sanitize_display_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(MAX_DISPLAY_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub fn default_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "Next Estudios", "Vellcast")
        .map(|d| d.config_dir().join("settings.json"))
}

#[derive(Debug, PartialEq)]
pub enum LoadOutcome {
    Loaded,
    Missing,
    /// Arquivo ilegível ou corrompido: os padrões são usados e o arquivo
    /// original é preservado com sufixo `.corrupt`.
    Recovered,
}

pub fn load(path: &Path) -> (Settings, LoadOutcome) {
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (Settings::default(), LoadOutcome::Missing);
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), %e, "não foi possível ler as configurações");
            return (Settings::default(), LoadOutcome::Recovered);
        }
    };
    match serde_json::from_slice::<Settings>(&raw) {
        Ok(s) => (s.sanitized(), LoadOutcome::Loaded),
        Err(e) => {
            tracing::warn!(path = %path.display(), %e, "configurações corrompidas; usando padrões");
            let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
            (Settings::default(), LoadOutcome::Recovered)
        }
    }
}

/// Grava de forma atômica (arquivo temporário + rename).
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("vellcast-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn defaults_are_safe() {
        let s = Settings::default();
        assert!(
            !s.background_shortcuts,
            "atalhos globais devem vir desligados"
        );
        assert!(!s.start_minimized);
        assert_eq!(s.language, Language::PtBr);
    }

    #[test]
    fn missing_file_uses_defaults() {
        let dir = temp_dir("missing");
        let (s, outcome) = load(&dir.join("settings.json"));
        assert_eq!(outcome, LoadOutcome::Missing);
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("settings.json");
        let s = Settings {
            language: Language::En,
            display_name: "Jogador".into(),
            gamepad_deadzone: 0.3,
            start_minimized: true,
            ..Settings::default()
        };
        save(&path, &s).expect("save");
        let (loaded, outcome) = load(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(loaded, s);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_file_is_preserved_and_defaults_used() {
        let dir = temp_dir("corrupt");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("settings.json");
        std::fs::write(&path, b"{ isto nao e json").expect("write");
        let (s, outcome) = load(&path);
        assert_eq!(outcome, LoadOutcome::Recovered);
        assert_eq!(s, Settings::default());
        assert!(dir.join("settings.json.corrupt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn partial_and_out_of_range_values_are_fixed() {
        let dir = temp_dir("partial");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{ "gamepad_deadzone": 3.0, "language": "en" }"#).expect("write");
        let (s, _) = load(&path);
        assert_eq!(s.language, Language::En);
        assert_eq!(s.gamepad_deadzone, 0.9);
        assert_eq!(s.gamepad_repeat_delay_ms, 400);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn display_name_is_sanitized() {
        let long = "a".repeat(80);
        assert_eq!(
            sanitize_display_name(&long).chars().count(),
            MAX_DISPLAY_NAME_CHARS
        );
        assert_eq!(sanitize_display_name("  Ana\u{0007}\n "), "Ana");
    }
}
