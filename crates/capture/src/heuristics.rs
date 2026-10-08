//! Heurística não invasiva para destacar janelas de jogos (S-003).
//!
//! Usa apenas dados públicos da janela (estilo, tamanho, executável). Não lê
//! memória de processos nem injeta código.

/// Executáveis comuns que não são jogos, mesmo em tela cheia.
const NOT_GAMES: &[&str] = &[
    "explorer.exe",
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "opera.exe",
    "brave.exe",
    "discord.exe",
    "code.exe",
    "devenv.exe",
    "obs64.exe",
    "vlc.exe",
    "applicationframehost.exe",
    "textinputhost.exe",
    "steamwebhelper.exe",
    "vellcast-desktop.exe",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowTraits<'a> {
    pub exe: Option<&'a str>,
    /// A janela cobre o monitor inteiro (tela cheia ou sem bordas).
    pub covers_monitor: bool,
    /// Tem barra de título (`WS_CAPTION`).
    pub has_caption: bool,
    pub width: u32,
    pub height: u32,
}

pub fn likely_game(t: WindowTraits<'_>) -> bool {
    if let Some(exe) = t.exe {
        let exe = exe.to_ascii_lowercase();
        if NOT_GAMES.iter().any(|n| *n == exe) {
            return false;
        }
    }
    let big_enough = t.width >= 1024 && t.height >= 576;
    big_enough && (t.covers_monitor || !t.has_caption)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn traits(exe: &str, covers: bool, caption: bool) -> WindowTraits<'_> {
        WindowTraits {
            exe: Some(exe),
            covers_monitor: covers,
            has_caption: caption,
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn borderless_fullscreen_window_is_game() {
        assert!(likely_game(traits("eldenring.exe", true, false)));
        assert!(likely_game(traits("game.exe", false, false)));
    }

    #[test]
    fn regular_windows_and_known_apps_are_not() {
        assert!(!likely_game(traits("notepad.exe", false, true)));
        assert!(!likely_game(traits("Chrome.exe", true, false)));
        let small = WindowTraits {
            width: 800,
            height: 600,
            ..traits("game.exe", false, false)
        };
        assert!(!likely_game(small));
    }
}
