//! Ícone na área de notificação (M-017).
//!
//! Os eventos chegam por callbacks (sem polling) e são levados à thread da
//! interface com `invoke_from_event_loop`.

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::settings::Language;

pub struct Tray {
    icon: TrayIcon,
    show: MenuItem,
    stop: MenuItem,
    quit: MenuItem,
}

pub enum TrayCommand {
    Show,
    /// Disponível só durante uma transmissão.
    StopStream,
    Quit,
}

fn labels(lang: Language) -> (&'static str, &'static str, &'static str) {
    match lang {
        Language::PtBr => ("Mostrar Vellcast", "Parar transmissão", "Sair"),
        Language::En => ("Show Vellcast", "Stop streaming", "Quit"),
    }
}

/// Dica do ícone: deixa claro quando a tela está sendo compartilhada (M-024).
fn tooltip(live: bool, lang: Language) -> &'static str {
    match (live, lang) {
        (false, _) => "Vellcast",
        (true, Language::PtBr) => "Vellcast — AO VIVO (compartilhando a tela)",
        (true, Language::En) => "Vellcast — LIVE (sharing your screen)",
    }
}

/// Ícone 32×32 gerado em código (quadrado arredondado na cor de destaque),
/// para não depender de decodificador de imagem.
fn icon() -> Result<Icon, tray_icon::BadIcon> {
    const S: u32 = 32;
    const R: f32 = 7.0;
    let mut rgba = Vec::with_capacity((S * S * 4) as usize);
    for y in 0..S {
        for x in 0..S {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(R, S as f32 - R);
            let cy = fy.clamp(R, S as f32 - R);
            let inside = (fx - cx).powi(2) + (fy - cy).powi(2) <= R * R;
            // "V" simples em #0E1014 sobre o teal #2EC4B6.
            let v = {
                let t = (fy - 9.0) / 14.0;
                (0.0..=1.0).contains(&t)
                    && ((fx - (9.0 + 7.0 * t)).abs() < 2.0 || (fx - (23.0 - 7.0 * t)).abs() < 2.0)
            };
            let px = match (inside, v) {
                (false, _) => [0, 0, 0, 0],
                (true, true) => [0x0E, 0x10, 0x14, 0xFF],
                (true, false) => [0x2E, 0xC4, 0xB6, 0xFF],
            };
            rgba.extend_from_slice(&px);
        }
    }
    Icon::from_rgba(rgba, S, S)
}

impl Tray {
    /// Cria o ícone. Deve rodar na thread da interface, com o loop de eventos ativo.
    pub fn create(
        lang: Language,
        on_command: impl Fn(TrayCommand) + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        let (show_label, stop_label, quit_label) = labels(lang);
        let show = MenuItem::new(show_label, true, None);
        let stop = MenuItem::new(stop_label, false, None);
        let quit = MenuItem::new(quit_label, true, None);
        let menu = Menu::new();
        menu.append(&show)?;
        menu.append(&stop)?;
        menu.append(&quit)?;

        let icon = TrayIconBuilder::new()
            .with_tooltip("Vellcast")
            .with_icon(icon()?)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;

        let on_command = std::sync::Arc::new(on_command);
        let (show_id, stop_id, quit_id) = (show.id().clone(), stop.id().clone(), quit.id().clone());
        let menu_cmd = on_command.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if e.id == show_id {
                menu_cmd(TrayCommand::Show);
            } else if e.id == stop_id {
                menu_cmd(TrayCommand::StopStream);
            } else if e.id == quit_id {
                menu_cmd(TrayCommand::Quit);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| match e {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick { .. } => on_command(TrayCommand::Show),
            _ => {}
        }));

        Ok(Self {
            icon,
            show,
            stop,
            quit,
        })
    }

    pub fn set_language(&self, lang: Language) {
        let (show_label, stop_label, quit_label) = labels(lang);
        self.show.set_text(show_label);
        self.stop.set_text(stop_label);
        self.quit.set_text(quit_label);
    }

    pub fn set_live(&self, live: bool, lang: Language) {
        self.stop.set_enabled(live);
        if let Err(e) = self.icon.set_tooltip(Some(tooltip(live, lang))) {
            tracing::debug!(%e, "não foi possível atualizar a dica da bandeja");
        }
    }
}
