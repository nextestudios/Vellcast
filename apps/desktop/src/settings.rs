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

#[derive(Clone, PartialEq, Serialize, Deserialize)]
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
    /// Endereço base do `vellcast-relay` (http:// ou https://).
    pub relay_url: String,
    /// Chave exigida pelo relay para criar salas (`VELLCAST_CREATE_ROOM_KEY`); vazia = sem chave.
    pub room_key: String,
    /// Endereço HTTPS usado nos links de convite quando o relay é acessado por
    /// outro endereço neste PC (ex.: túnel ou proxy). Vazio = mesmo endereço do servidor.
    pub public_url: String,
    /// Preferências da tela Transmitir.
    pub stream: StreamPrefs,
    /// Versão do formato, para migrações únicas. Arquivos antigos não têm o
    /// campo e leem 0 (o padrão do campo, não o de [`Settings::default`]).
    #[serde(default)]
    pub config_version: u32,
}

/// Versão atual de [`Settings::config_version`].
/// 1 → 2: o servidor padrão passou do relay local para o servidor de testes.
const CONFIG_VERSION: u32 = 2;

// Debug manual: a chave de criação de salas nunca aparece em logs.
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("language", &self.language)
            .field("relay_url", &self.relay_url)
            .field("public_url", &self.public_url)
            .field(
                "room_key",
                &if self.room_key.is_empty() {
                    ""
                } else {
                    "<oculta>"
                },
            )
            .field("stream", &self.stream)
            .finish_non_exhaustive()
    }
}

/// Servidor de teste do MVP (relay hospedado com TLS). Para usar um relay neste
/// PC, configure `http://127.0.0.1:8788`: o app o inicia sozinho (`local_relay`).
pub const DEFAULT_RELAY_URL: &str = "https://vellcast.nextestudios.com";
/// Padrão das versões anteriores; migrado para [`DEFAULT_RELAY_URL`].
const LEGACY_DEFAULT_RELAY_URL: &str = "http://127.0.0.1:8788";

/// Preferências de qualidade e áudio (índices dos seletores da interface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StreamPrefs {
    /// 0 Econômico, 1 Equilibrado, 2 Qualidade, 3 Competitivo, 4 Personalizado.
    pub profile: u8,
    /// Altura máxima no modo Personalizado: 720, 900 ou 1080.
    pub custom_height: u32,
    pub custom_fps: u32,
    pub custom_bitrate_kbps: u32,
    /// 0 desligado, 1 aplicativo, 2 sistema inteiro.
    pub audio: u8,
    /// Nome do encoder escolhido; vazio = automático.
    pub encoder: String,
}

impl Default for StreamPrefs {
    fn default() -> Self {
        Self {
            profile: 1,
            custom_height: 1080,
            custom_fps: 60,
            custom_bitrate_kbps: 8000,
            audio: 1,
            encoder: String::new(),
        }
    }
}

pub const CUSTOM_HEIGHTS: [u32; 3] = [720, 900, 1080];
pub const CUSTOM_FPS: [u32; 2] = [30, 60];

impl StreamPrefs {
    pub fn sanitized(mut self) -> Self {
        self.profile = self.profile.min(4);
        self.audio = self.audio.min(2);
        if !CUSTOM_HEIGHTS.contains(&self.custom_height) {
            self.custom_height = 1080;
        }
        if !CUSTOM_FPS.contains(&self.custom_fps) {
            self.custom_fps = 60;
        }
        // Passos de 500 kbps entre 1 e 20 Mbps (limites de vellcast_core::Quality).
        self.custom_bitrate_kbps = (self.custom_bitrate_kbps.clamp(1000, 20000) / 500) * 500;
        self.encoder = self
            .encoder
            .chars()
            .filter(|c| !c.is_control())
            .take(128)
            .collect();
        self
    }

    /// Qualidade efetiva (o plano gratuito vai até 1080p60, sem restrição artificial).
    pub fn quality(&self) -> vellcast_core::Quality {
        use vellcast_core::{Quality, QualityProfile};
        let profile = match self.profile {
            0 => QualityProfile::Economico,
            2 => QualityProfile::Qualidade,
            3 => QualityProfile::Competitivo,
            4 => QualityProfile::Personalizado,
            _ => QualityProfile::Equilibrado,
        };
        if profile != QualityProfile::Personalizado {
            return profile.quality();
        }
        Quality {
            max_width: self.custom_height * 16 / 9,
            max_height: self.custom_height,
            fps: self.custom_fps,
            bitrate_kbps: self.custom_bitrate_kbps,
        }
        .clamped()
    }

    pub fn audio_mode(&self) -> vellcast_core::AudioMode {
        match self.audio {
            0 => vellcast_core::AudioMode::Off,
            2 => vellcast_core::AudioMode::System,
            _ => vellcast_core::AudioMode::Application,
        }
    }
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
            relay_url: DEFAULT_RELAY_URL.into(),
            room_key: String::new(),
            public_url: String::new(),
            stream: StreamPrefs::default(),
            config_version: CONFIG_VERSION,
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
        self.relay_url = self.relay_url.trim().chars().take(256).collect();
        // Migração única: quem tinha o padrão antigo passa para o servidor de testes;
        // escolher o relay local depois disso continua valendo.
        if self.config_version < CONFIG_VERSION && self.relay_url == LEGACY_DEFAULT_RELAY_URL {
            self.relay_url = DEFAULT_RELAY_URL.into();
        }
        self.config_version = CONFIG_VERSION;
        if self.relay_url.is_empty() {
            self.relay_url = DEFAULT_RELAY_URL.into();
        }
        self.room_key = self
            .room_key
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(256)
            .collect();
        self.public_url = self.public_url.trim().chars().take(256).collect();
        self.stream = self.stream.sanitized();
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
    fn old_files_get_relay_and_stream_defaults() {
        let dir = temp_dir("old");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("settings.json");
        std::fs::write(&path, br#"{ "language": "pt-BR", "display_name": "Ana" }"#).expect("write");
        let (s, outcome) = load(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(s.relay_url, DEFAULT_RELAY_URL);
        assert_eq!(s.room_key, "");
        assert_eq!(s.stream, StreamPrefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn migrates_old_default_server_only_once() {
        let dir = std::env::temp_dir().join(format!("vellcast-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("settings.json");
        // Arquivo de uma versão anterior: sem config_version e com o padrão antigo.
        std::fs::write(&path, br#"{ "relay_url": "http://127.0.0.1:8788" }"#).expect("write");
        let (s, _) = load(&path);
        assert_eq!(s.relay_url, DEFAULT_RELAY_URL);
        assert_eq!(s.config_version, CONFIG_VERSION);
        // Depois da migração, escolher o relay local é respeitado.
        let chosen = Settings {
            relay_url: LEGACY_DEFAULT_RELAY_URL.into(),
            ..s
        }
        .sanitized();
        assert_eq!(chosen.relay_url, LEGACY_DEFAULT_RELAY_URL);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stream_prefs_are_clamped_and_never_exceed_1080p60() {
        let p = StreamPrefs {
            profile: 9,
            custom_height: 2160,
            custom_fps: 144,
            custom_bitrate_kbps: 99_999,
            audio: 7,
            encoder: "x".repeat(300),
        }
        .sanitized();
        assert_eq!((p.profile, p.audio), (4, 2));
        assert_eq!(
            (p.custom_height, p.custom_fps, p.custom_bitrate_kbps),
            (1080, 60, 20000)
        );
        assert_eq!(p.encoder.chars().count(), 128);
        let q = p.quality();
        assert_eq!((q.max_width, q.max_height, q.fps), (1920, 1080, 60));
        let p720 = StreamPrefs {
            profile: 4,
            custom_height: 720,
            custom_fps: 30,
            custom_bitrate_kbps: 2500,
            ..StreamPrefs::default()
        };
        let q = p720.quality();
        assert_eq!(
            (q.max_width, q.max_height, q.fps, q.bitrate_kbps),
            (1280, 720, 30, 2500)
        );
        // Perfil predefinido ignora os campos personalizados.
        assert_eq!(
            StreamPrefs { profile: 1, ..p720 }.quality(),
            vellcast_core::QualityProfile::Equilibrado.quality()
        );
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
