//! Vellcast desktop: interface, navegação por controle, bandeja, salas e transmissão.
//!
//! Modelo de threads: a interface roda na thread principal (loop do Slint).
//! A entrada de gamepad roda em `vellcast-gamepad`, o motor de transmissão em
//! `vellcast-engine` e as chamadas HTTP ao relay em threads curtas; todos só
//! entregam resultados à interface via `invoke_from_event_loop`. Não há polling
//! nem animações contínuas, então a janela oculta não renderiza nada.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod actions;
mod engine;
mod input;
mod local_relay;
mod procstats;
mod relay_api;
mod rooms;
mod settings;
mod tray;
mod update_ui;
mod updater;

#[cfg(feature = "dev-snapshot")]
mod dev;
#[cfg(test)]
mod i18n_tests;

use std::cell::RefCell;
use std::path::PathBuf;

use slint::platform::{Key, WindowEvent};
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use vellcast_controller::{Action, ControllerEvent, ControllerHandle};

use crate::input::{History, ModalAction, Route, SyntheticKey};
use crate::settings::{Language, Settings};
use crate::tray::{Tray, TrayCommand};

/// Código gerado pelo Slint a partir de `ui/` (usa `unwrap` internamente).
#[allow(clippy::unwrap_used)]
mod ui {
    slint::include_modules!();
}
use ui::{AppWindow, InputState, NavAction, TextOps};

/// Licenças exibidas na tela Sobre (inventário completo em docs/LICENSING.md).
const LICENSES: &[&str] = &[
    "Slint 1.18 — Slint Royalty-free License 2.0",
    "GilRs 0.11 — Apache-2.0 / MIT",
    "tray-icon 0.24 — MIT / Apache-2.0",
    "Lucide icons — ISC",
    "serde, serde_json, anyhow, directories — MIT / Apache-2.0",
    "ureq 3, rustls, arboard — MIT / Apache-2.0",
    "ring — Apache-2.0 AND ISC; webpki-roots — CDLA-Permissive-2.0",
    "tracing, tracing-subscriber — MIT",
];

/// Estado que vive apenas na thread da interface.
pub(crate) struct UiState {
    settings: Settings,
    settings_path: Option<PathBuf>,
    history: History,
    pads: Vec<(usize, String)>,
    controller: Option<ControllerHandle>,
    tray: Option<Tray>,
    window_visible: bool,
    rooms: rooms::RoomBook,
    rooms_path: Option<PathBuf>,
    sources: Vec<vellcast_core::SourceInfo>,
    encoders: Vec<vellcast_core::EncoderInfo>,
    engine: Option<engine::Engine>,
    live: Option<actions::LiveSession>,
    live_timer: Option<slint::Timer>,
}

thread_local! {
    static STATE: RefCell<Option<UiState>> = const { RefCell::new(None) };
}

pub(crate) fn with_state<R>(f: impl FnOnce(&mut UiState) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,vellcast_desktop=debug".into()),
        )
        .init();

    let settings_path = settings::default_path();
    let settings = match &settings_path {
        Some(path) => {
            let (s, outcome) = settings::load(path);
            tracing::info!(path = %path.display(), ?outcome, "configurações carregadas");
            s
        }
        None => {
            tracing::warn!("diretório de configuração indisponível; usando padrões sem salvar");
            Settings::default()
        }
    };

    let rooms_path = settings_path.as_deref().map(rooms::path_next_to);
    let room_book = rooms_path
        .as_deref()
        .map(|p| rooms::load(p, actions::now_unix()))
        .unwrap_or_default();

    let ui = AppWindow::new()?;
    apply_settings_to_ui(&ui, &settings);
    select_language(settings.language);
    ui.set_version(env!("CARGO_PKG_VERSION").into());
    ui.set_licenses(string_model(LICENSES.iter().map(|s| s.to_string())));
    ui.set_gamepads(string_model(std::iter::empty()));

    let text_ops = ui.global::<TextOps>();
    text_ops.on_pop_char(|s| input::pop_char(&s).into());
    text_ops.on_to_upper(|s| s.to_uppercase().into());

    wire_callbacks(&ui);

    let controller = start_controller(&ui, &settings);
    track_window_focus(&ui);

    let start_hidden = settings.start_minimized;
    STATE.with(|s| {
        *s.borrow_mut() = Some(UiState {
            settings,
            settings_path,
            history: History::default(),
            pads: Vec::new(),
            controller,
            tray: None,
            window_visible: false,
            rooms: room_book,
            rooms_path,
            sources: Vec::new(),
            encoders: Vec::new(),
            engine: None,
            live: None,
            live_timer: None,
        });
    });
    actions::init(&ui);

    // A bandeja precisa do loop de eventos já ativo (exigência do macOS).
    let weak = ui.as_weak();
    slint::invoke_from_event_loop(move || {
        let Some(ui) = weak.upgrade() else { return };
        let tray_ok = create_tray(&ui);
        ui.set_tray_available(tray_ok);
        tracing::info!(start_hidden, tray_ok, "estado inicial da janela");
        if !(start_hidden && tray_ok) {
            show_window(&ui);
        }
        #[cfg(feature = "dev-snapshot")]
        dev::schedule(&ui);
    })?;

    // Continua rodando com a janela oculta na bandeja.
    slint::run_event_loop_until_quit()?;
    // Encerra a transmissão (se houver) antes de sair: o motor para a sessão no Drop.
    STATE.with(|s| drop(s.borrow_mut().take()));
    // Desliga o servidor local, se o app o iniciou.
    local_relay::shutdown();
    Ok(())
}

fn string_model(items: impl Iterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items.map(SharedString::from).collect::<Vec<_>>(),
    ))
}

fn apply_settings_to_ui(ui: &AppWindow, s: &Settings) {
    ui.set_language_index(s.language.index());
    ui.set_display_name(s.display_name.clone().into());
    ui.set_deadzone(s.gamepad_deadzone);
    ui.set_repeat_delay(s.gamepad_repeat_delay_ms as f32);
    ui.set_repeat_interval(s.gamepad_repeat_interval_ms as f32);
    ui.set_start_minimized(s.start_minimized);
    ui.set_close_to_tray(s.close_to_tray);
    ui.set_background_shortcuts(s.background_shortcuts);
    ui.set_relay_url(s.relay_url.clone().into());
    ui.set_room_key(s.room_key.clone().into());
    ui.set_public_url(s.public_url.clone().into());
}

fn settings_from_ui(
    ui: &AppWindow,
    previous: &Settings,
    encoders: &[vellcast_core::EncoderInfo],
) -> Settings {
    Settings {
        language: Language::from_index(ui.get_language_index()),
        display_name: ui.get_display_name().to_string(),
        gamepad_deadzone: ui.get_deadzone(),
        gamepad_repeat_delay_ms: ui.get_repeat_delay().round().max(0.0) as u64,
        gamepad_repeat_interval_ms: ui.get_repeat_interval().round().max(0.0) as u64,
        start_minimized: ui.get_start_minimized(),
        close_to_tray: ui.get_close_to_tray(),
        background_shortcuts: previous.background_shortcuts,
        relay_url: ui.get_relay_url().to_string(),
        room_key: ui.get_room_key().to_string(),
        public_url: ui.get_public_url().to_string(),
        stream: actions::stream_prefs_from_ui(ui, &previous.stream, encoders),
        config_version: previous.config_version,
    }
    .sanitized()
}

fn select_language(lang: Language) {
    // O idioma-fonte das strings é o inglês; pt-BR vem do catálogo embutido.
    let code = match lang {
        Language::PtBr => "pt-BR",
        Language::En => "",
    };
    if let Err(e) = slint::select_bundled_translation(code) {
        tracing::warn!(lang = lang.code(), %e, "tradução indisponível");
    }
}

pub(crate) fn persist(ui: &AppWindow) {
    with_state(|st| {
        let new = settings_from_ui(ui, &st.settings, &st.encoders);
        if new == st.settings {
            return;
        }
        if let Some(c) = &st.controller {
            c.set_config(new.input_config());
        }
        if let Some(path) = &st.settings_path
            && let Err(e) = settings::save(path, &new)
        {
            tracing::warn!(%e, "não foi possível salvar as configurações");
        }
        // Reflete correções (ex.: nome sanitizado) de volta na interface.
        if new.display_name != ui.get_display_name().as_str() {
            ui.set_display_name(new.display_name.clone().into());
        }
        if new.relay_url != ui.get_relay_url().as_str() {
            ui.set_relay_url(new.relay_url.clone().into());
        }
        st.settings = new;
    });
}

/// Troca de tela registrando o histórico (Voltar) e ajustando o timer da tela Ao vivo.
pub(crate) fn navigate_to(ui: &AppWindow, page: i32) {
    let current = ui.get_current_page();
    if current != page {
        with_state(|st| st.history.push(current, page));
        ui.set_current_page(page);
    }
    actions::update_live_timer(ui);
}

fn wire_callbacks(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_navigate(move |page| {
        if let Some(ui) = weak.upgrade() {
            navigate_to(&ui, page);
        }
    });

    let weak = ui.as_weak();
    ui.on_settings_changed(move || {
        if let Some(ui) = weak.upgrade() {
            persist(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_language_selected(move |index| {
        let lang = Language::from_index(index);
        select_language(lang);
        with_state(|st| {
            if let Some(t) = &st.tray {
                t.set_language(lang);
            }
        });
        if let Some(ui) = weak.upgrade() {
            persist(&ui);
            actions::on_language_changed(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_minimize_to_tray(move || {
        if let Some(ui) = weak.upgrade() {
            hide_window(&ui);
        }
    });

    ui.on_quit_app(|| {
        let _ = slint::quit_event_loop();
    });

    update_ui::wire(ui);

    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        let to_tray =
            with_state(|st| st.tray.is_some() && st.settings.close_to_tray).unwrap_or(false);
        if to_tray {
            if let Some(ui) = weak.upgrade() {
                hide_window(&ui);
            }
        } else {
            let _ = slint::quit_event_loop();
        }
        slint::CloseRequestResponse::HideWindow
    });
}

fn create_tray(ui: &AppWindow) -> bool {
    let lang = with_state(|st| st.settings.language).unwrap_or_default();
    let weak = ui.as_weak();
    match Tray::create(lang, move |cmd| {
        let weak = weak.clone();
        let _ = slint::invoke_from_event_loop(move || match cmd {
            TrayCommand::Show => {
                if let Some(ui) = weak.upgrade() {
                    show_window(&ui);
                }
            }
            TrayCommand::StopStream => {
                if let Some(ui) = weak.upgrade() {
                    actions::stop_stream(&ui, None);
                }
            }
            TrayCommand::Quit => {
                let _ = slint::quit_event_loop();
            }
        });
    }) {
        Ok(tray) => {
            with_state(|st| st.tray = Some(tray));
            tracing::info!("ícone da bandeja criado");
            true
        }
        Err(e) => {
            tracing::warn!(%e, "ícone da bandeja indisponível");
            false
        }
    }
}

fn show_window(ui: &AppWindow) {
    if let Err(e) = ui.show() {
        tracing::error!(%e, "falha ao mostrar a janela");
        return;
    }
    ui.window().with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
    with_state(|st| st.window_visible = true);
    actions::update_live_timer(ui);
}

fn hide_window(ui: &AppWindow) {
    if with_state(|st| st.tray.is_none()).unwrap_or(true) {
        // Sem bandeja, apenas minimiza para não perder o acesso ao app.
        ui.window().set_minimized(true);
        return;
    }
    let _ = ui.hide();
    with_state(|st| {
        st.window_visible = false;
        if let Some(c) = &st.controller {
            c.set_active(false);
        }
    });
    // Oculto na bandeja: a transmissão continua, as métricas da interface param.
    actions::update_live_timer(ui);
}

/// O controle só age com a janela do Vellcast em foco (spec §13).
/// Também recolhe a lista de salas em janelas estreitas.
fn track_window_focus(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |window, event| {
        match event {
            winit::event::WindowEvent::Focused(focused) => {
                with_state(|st| {
                    if let Some(c) = &st.controller {
                        c.set_active(*focused && st.window_visible);
                    }
                });
            }
            winit::event::WindowEvent::Resized(size) => {
                let width = size.width as f32 / window.scale_factor();
                if let Some(ui) = weak.upgrade() {
                    ui.set_rooms_collapsed(width < 1200.0);
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn start_controller(ui: &AppWindow, settings: &Settings) -> Option<ControllerHandle> {
    let weak = ui.as_weak();
    match vellcast_controller::spawn(settings.input_config(), move |event| {
        let _ = weak.upgrade_in_event_loop(move |ui| on_controller_event(&ui, event));
    }) {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!(%e, "gamepad indisponível; mouse e teclado continuam funcionando");
            None
        }
    }
}

fn on_controller_event(ui: &AppWindow, event: ControllerEvent) {
    match event {
        ControllerEvent::Connected { id, name } => {
            tracing::info!(id, %name, "controle conectado");
            update_pads(ui, |pads| {
                pads.retain(|(i, _)| *i != id);
                pads.push((id, name));
            });
        }
        ControllerEvent::Disconnected { id } => {
            tracing::info!(id, "controle desconectado");
            update_pads(ui, |pads| pads.retain(|(i, _)| *i != id));
        }
        ControllerEvent::Action(action) => handle_action(ui, action),
    }
}

fn update_pads(ui: &AppWindow, f: impl FnOnce(&mut Vec<(usize, String)>)) {
    let names = with_state(|st| {
        f(&mut st.pads);
        st.pads.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>()
    })
    .unwrap_or_default();
    ui.set_gamepads(string_model(names.into_iter()));
}

fn handle_action(ui: &AppWindow, action: Action) {
    let input_state = ui.global::<InputState>();
    input_state.set_gamepad(true);
    match input::route(action, ui.get_modal() != 0) {
        Route::Key(key) => press_key(ui, key),
        Route::Modal(a) => ui.invoke_modal_action(nav_action(a)),
        Route::HistoryBack => {
            if let Some(page) = with_state(|st| st.history.back()).flatten() {
                ui.set_current_page(page);
                ui.set_nav_focus_token(ui.get_nav_focus_token() + 1);
                actions::update_live_timer(ui);
            }
        }
        Route::Section(delta) => {
            let current = ui.get_current_page();
            let next = input::step_section(current, delta, ui.get_page_count());
            with_state(|st| st.history.push(current, next));
            ui.set_current_page(next);
            ui.set_nav_focus_token(ui.get_nav_focus_token() + 1);
            actions::update_live_timer(ui);
        }
        Route::Tab(delta) => ui.invoke_tab_step(delta),
        Route::Scroll(delta) => ui.invoke_scroll_step(delta),
        Route::OpenMenu => ui.invoke_open_menu(),
        Route::Unassigned => tracing::debug!(?action, "ação sem função nesta tela"),
    }
    input_state.set_gamepad(false);
}

fn nav_action(a: ModalAction) -> NavAction {
    match a {
        ModalAction::Up => NavAction::Up,
        ModalAction::Down => NavAction::Down,
        ModalAction::Left => NavAction::Left,
        ModalAction::Right => NavAction::Right,
        ModalAction::Confirm => NavAction::Confirm,
        ModalAction::Back => NavAction::Back,
        ModalAction::Secondary => NavAction::Secondary,
        ModalAction::Options => NavAction::Options,
        ModalAction::Menu => NavAction::Menu,
    }
}

fn press_key(ui: &AppWindow, key: SyntheticKey) {
    let text: SharedString = match key {
        SyntheticKey::Tab => Key::Tab.into(),
        SyntheticKey::Backtab => Key::Backtab.into(),
        SyntheticKey::Left => Key::LeftArrow.into(),
        SyntheticKey::Right => Key::RightArrow.into(),
        SyntheticKey::Return => Key::Return.into(),
    };
    let window = ui.window();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}
