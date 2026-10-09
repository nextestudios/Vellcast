//! Captura da própria janela para revisão visual (feature `dev-snapshot`).
//!
//! `VELLCAST_DEV_SNAPSHOT=saida.bmp` grava a janela e encerra o app.
//! Opcionais: `VELLCAST_DEV_PAGE=<0-7>`, `VELLCAST_DEV_MODAL=menu|keyboard`,
//! `VELLCAST_DEV_TABS=<n>` (envia n Tabs para mostrar o anel de foco),
//! `VELLCAST_DEV_ACTIONS=down,down,confirm,...` (ações de controle simuladas,
//! processadas pelo mesmo caminho do gamepad real) e `VELLCAST_DEV_WAIT_MS=<ms>`
//! (espera antes da captura, para resultados assíncronos) e `VELLCAST_DEV_AUTOSTREAM=1`
//! (cria sala e transmite o primeiro monitor; usado pelo E2E no Windows).

use std::time::Duration;

use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Timer};

use vellcast_controller::Action;

use crate::ui::AppWindow;

pub fn schedule(ui: &AppWindow) {
    let Ok(path) = std::env::var("VELLCAST_DEV_SNAPSHOT") else {
        return;
    };
    if std::env::var("VELLCAST_DEV_AUTOSTREAM").as_deref() == Ok("1") {
        crate::actions::dev_autostream(ui);
    }
    let weak = ui.as_weak();
    Timer::single_shot(Duration::from_millis(600), move || {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(page) = std::env::var("VELLCAST_DEV_PAGE")
            .ok()
            .and_then(|p| p.parse().ok())
        {
            ui.set_current_page(page);
        }
        let tabs: usize = std::env::var("VELLCAST_DEV_TABS")
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        for _ in 0..tabs {
            ui.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Tab.into(),
            });
            ui.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Tab.into(),
            });
        }
        if let Ok(list) = std::env::var("VELLCAST_DEV_ACTIONS") {
            for name in list.split(',') {
                match parse_action(name.trim()) {
                    Some(a) => crate::handle_action(&ui, a),
                    None => tracing::warn!(name, "ação desconhecida"),
                }
            }
        }
        match std::env::var("VELLCAST_DEV_MODAL").as_deref() {
            Ok("menu") => ui.invoke_open_menu(),
            Ok("keyboard") => ui.invoke_open_keyboard(),
            _ => {}
        }
        // `VELLCAST_DEV_WAIT_MS`: espera extra antes da captura, para ações assíncronas
        // (ex.: criar sala, que pode iniciar o servidor local).
        let wait = std::env::var("VELLCAST_DEV_WAIT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);
        let weak = ui.as_weak();
        Timer::single_shot(Duration::from_millis(wait), move || {
            let Some(ui) = weak.upgrade() else { return };
            let size = ui.window().size();
            tracing::info!(
                w = size.width,
                h = size.height,
                scale = ui.window().scale_factor(),
                "tamanho da janela"
            );
            match ui.window().take_snapshot() {
                Ok(buf) => {
                    if let Err(e) = write_bmp(&path, buf.width(), buf.height(), buf.as_bytes()) {
                        tracing::error!(%e, "falha ao gravar o snapshot");
                    }
                }
                Err(e) => tracing::error!(%e, "falha ao capturar a janela"),
            }
            let _ = slint::quit_event_loop();
        });
    });
}

fn parse_action(name: &str) -> Option<Action> {
    Some(match name {
        "up" => Action::Up,
        "down" => Action::Down,
        "left" => Action::Left,
        "right" => Action::Right,
        "confirm" => Action::Confirm,
        "back" => Action::Back,
        "secondary" => Action::Secondary,
        "options" => Action::Options,
        "lb" => Action::PrevSection,
        "rb" => Action::NextSection,
        "lt" => Action::PrevTab,
        "rt" => Action::NextTab,
        "scroll_up" => Action::ScrollUp,
        "scroll_down" => Action::ScrollDown,
        "menu" => Action::Menu,
        _ => return None,
    })
}

/// BMP 32 bits sem compressão (BGRA, linhas de baixo para cima).
fn write_bmp(path: &str, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let row = (width * 4) as usize;
    let data_len = row * height as usize;
    let mut out = Vec::with_capacity(54 + data_len);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for y in (0..height as usize).rev() {
        for px in rgba[y * row..(y + 1) * row].as_chunks::<4>().0 {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    std::fs::write(path, out)
}
