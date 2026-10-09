//! Fluxos da interface: salas, convites, transmissão e tela Ao vivo.
//!
//! Regras (spec §12): rede e motor nunca rodam na thread da interface; as
//! métricas só são atualizadas com a tela Ao vivo visível; nada é capturado
//! sem o clique em "Iniciar transmissão" (M-024).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use vellcast_core::{
    AudioMode, EncoderInfo, EncoderPreference, SessionConfig, SessionEvent, SessionStats,
    SourceInfo, SourceKind,
};

use crate::engine::{Command, Engine, EngineMsg};
use crate::relay_api::{self, ApiError};
use crate::rooms::{self, SavedRoom};
use crate::settings::{CUSTOM_FPS, CUSTOM_HEIGHTS, Language, StreamPrefs};
use crate::ui::{AppWindow, RoomItem, SourceItem};
use crate::with_state;

/// Índice da tela Ao vivo na barra lateral.
pub const LIVE_PAGE: i32 = 3;
const TICK: Duration = Duration::from_secs(1);
/// A contagem de espectadores vem do relay a cada N ticks (baixa frequência).
const VIEWER_POLL_TICKS: u32 = 3;

/// Transmissão em andamento (estado da interface).
pub struct LiveSession {
    pub room_id: String,
    pub relay: String,
    pub owner_token: String,
    pub started: Option<Instant>,
    pub ticks: u32,
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn en() -> bool {
    with_state(|st| st.settings.language == Language::En).unwrap_or(false)
}

/// Texto da interface gerado em Rust, no idioma atual.
fn t(pt: &str, en_text: &str) -> SharedString {
    if en() { en_text.into() } else { pt.into() }
}

/// Executa `work` em uma thread de fundo e entrega o resultado na thread da interface.
fn background<T: Send + 'static>(
    ui: &AppWindow,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&AppWindow, T) + Send + 'static,
) {
    let weak = ui.as_weak();
    let spawned = std::thread::Builder::new()
        .name("vellcast-http".into())
        .spawn(move || {
            let result = work();
            let _ = weak.upgrade_in_event_loop(move |ui| done(&ui, result));
        });
    if let Err(e) = spawned {
        tracing::error!(%e, "não foi possível criar thread de fundo");
    }
}

// ---------------------------------------------------------------------------
// Inicialização

pub fn init(ui: &AppWindow) {
    let weak = ui.as_weak();
    let engine = Engine::spawn(move |msg| {
        let _ = weak.upgrade_in_event_loop(move |ui| on_engine_msg(&ui, msg));
    });
    match engine {
        Ok(engine) => {
            engine.send(Command::ListEncoders);
            engine.send(Command::ListSources);
            ui.set_sources_loading(true);
            with_state(|st| st.engine = Some(engine));
        }
        Err(e) => tracing::error!(%e, "motor de transmissão indisponível"),
    }

    let timer = Timer::default();
    with_state(|st| st.live_timer = Some(timer));

    wire(ui);
    apply_stream_prefs(ui);
    refresh_rooms_ui(ui);
    set_encoder_options(ui, &[]);
}

fn wire(ui: &AppWindow) {
    let w = ui.as_weak();
    ui.on_refresh_sources(move || {
        if let Some(ui) = w.upgrade() {
            refresh_sources(&ui);
        }
    });
    let w = ui.as_weak();
    ui.on_stream_prefs_changed(move || {
        if let Some(ui) = w.upgrade() {
            update_quality_summary(&ui);
            crate::persist(&ui);
        }
    });
    let w = ui.as_weak();
    ui.on_start_stream(move || {
        if let Some(ui) = w.upgrade() {
            start_stream(&ui);
        }
    });
    let w = ui.as_weak();
    ui.on_stop_stream(move || {
        if let Some(ui) = w.upgrade() {
            stop_stream(&ui, None);
        }
    });
    let w = ui.as_weak();
    ui.on_create_room(move |name| {
        if let Some(ui) = w.upgrade() {
            create_room(&ui, name.as_str());
        }
    });
    let w = ui.as_weak();
    ui.on_make_invite(move |index| {
        if let Some(ui) = w.upgrade() {
            let id =
                with_state(|st| st.rooms.rooms.get(index as usize).map(|r| r.id.clone())).flatten();
            if let Some(id) = id {
                make_invite(&ui, id);
            }
        }
    });
    let w = ui.as_weak();
    ui.on_make_live_invite(move || {
        if let Some(ui) = w.upgrade() {
            let id = with_state(|st| st.live.as_ref().map(|l| l.room_id.clone())).flatten();
            if let Some(id) = id {
                make_invite(&ui, id);
            }
        }
    });
    let w = ui.as_weak();
    ui.on_make_pairing(move |index| {
        if let Some(ui) = w.upgrade() {
            let id =
                with_state(|st| st.rooms.rooms.get(index as usize).map(|r| r.id.clone())).flatten();
            if let Some(id) = id {
                make_pairing(&ui, id);
            }
        }
    });
    let w = ui.as_weak();
    ui.on_copy_invite(move || {
        if let Some(ui) = w.upgrade() {
            copy_invite(&ui);
        }
    });
    let w = ui.as_weak();
    ui.on_close_room(move |index| {
        if let Some(ui) = w.upgrade() {
            close_room(&ui, index);
        }
    });
    let w = ui.as_weak();
    ui.on_test_relay(move || {
        if let Some(ui) = w.upgrade() {
            test_relay(&ui);
        }
    });
}

// ---------------------------------------------------------------------------
// Preferências de transmissão

pub fn apply_stream_prefs(ui: &AppWindow) {
    let Some(p) = with_state(|st| st.settings.stream.clone()) else {
        return;
    };
    ui.set_profile(i32::from(p.profile));
    ui.set_custom_height_index(
        CUSTOM_HEIGHTS
            .iter()
            .position(|h| *h == p.custom_height)
            .unwrap_or(2) as i32,
    );
    ui.set_custom_fps_index(
        CUSTOM_FPS
            .iter()
            .position(|f| *f == p.custom_fps)
            .unwrap_or(1) as i32,
    );
    ui.set_custom_bitrate(p.custom_bitrate_kbps as f32);
    ui.set_audio_index(i32::from(p.audio));
    update_quality_summary(ui);
}

/// Lê os seletores da tela Transmitir (o encoder vem do estado).
pub fn stream_prefs_from_ui(
    ui: &AppWindow,
    previous: &StreamPrefs,
    encoders: &[EncoderInfo],
) -> StreamPrefs {
    let index = ui.get_encoder_index();
    let encoder = if index <= 0 {
        String::new()
    } else {
        encoders
            .get(index as usize - 1)
            .map_or_else(|| previous.encoder.clone(), |e| e.name.clone())
    };
    StreamPrefs {
        profile: ui.get_profile().clamp(0, 4) as u8,
        custom_height: CUSTOM_HEIGHTS[ui.get_custom_height_index().clamp(0, 2) as usize],
        custom_fps: CUSTOM_FPS[ui.get_custom_fps_index().clamp(0, 1) as usize],
        custom_bitrate_kbps: ui.get_custom_bitrate().round().max(0.0) as u32,
        audio: ui.get_audio_index().clamp(0, 2) as u8,
        encoder,
    }
    .sanitized()
}

fn update_quality_summary(ui: &AppWindow) {
    let encoders = with_state(|st| st.encoders.clone()).unwrap_or_default();
    let previous = with_state(|st| st.settings.stream.clone()).unwrap_or_default();
    let q = stream_prefs_from_ui(ui, &previous, &encoders).quality();
    // Consumo de banda por espectador: kbps × 3600 s ÷ 8 ÷ 10⁶ = GB/h.
    let gb_per_hour = f64::from(q.bitrate_kbps) * 3600.0 / 8.0 / 1_000_000.0;
    let text = if en() {
        format!(
            "Up to {}×{} at {} FPS, {} kbps · about {:.1} GB per hour for each viewer",
            q.max_width, q.max_height, q.fps, q.bitrate_kbps, gb_per_hour
        )
    } else {
        format!(
            "Até {}×{} a {} FPS, {} kbps · cerca de {:.1} GB por hora para cada espectador",
            q.max_width, q.max_height, q.fps, q.bitrate_kbps, gb_per_hour
        )
        .replace('.', ",")
    };
    ui.set_quality_summary(text.into());
}

fn set_encoder_options(ui: &AppWindow, encoders: &[EncoderInfo]) {
    let auto = t("Automático", "Automatic");
    let mut options = vec![auto];
    for e in encoders {
        let kind = if e.hardware {
            t("hardware", "hardware")
        } else {
            t("software", "software")
        };
        options.push(format!("{} ({kind})", e.name).into());
    }
    let preferred = with_state(|st| st.settings.stream.encoder.clone()).unwrap_or_default();
    let index = encoders
        .iter()
        .position(|e| e.name == preferred)
        .map_or(0, |i| i as i32 + 1);
    ui.set_encoder_options(ModelRc::new(VecModel::from(options)));
    ui.set_encoder_index(index);
}

// ---------------------------------------------------------------------------
// Fontes de captura

fn refresh_sources(ui: &AppWindow) {
    ui.set_sources_loading(true);
    ui.set_sources_message(SharedString::new());
    with_state(|st| {
        if let Some(e) = &st.engine {
            e.send(Command::ListSources);
        }
    });
}

fn source_detail(s: &SourceInfo) -> String {
    let kind = match s.kind {
        SourceKind::Window => t("Janela", "Window"),
        SourceKind::Monitor => t("Monitor", "Monitor"),
    };
    match &s.process_name {
        Some(p) => format!("{kind} · {p} · {}×{}", s.width, s.height),
        None => format!("{kind} · {}×{}", s.width, s.height),
    }
}

fn set_sources(ui: &AppWindow, result: Result<Vec<SourceInfo>, String>) {
    ui.set_sources_loading(false);
    let previous = with_state(|st| {
        let i = ui.get_selected_source();
        (i >= 0)
            .then(|| st.sources.get(i as usize).map(|s| s.id))
            .flatten()
    })
    .flatten();
    let mut list = match result {
        Ok(list) => list,
        Err(e) => {
            ui.set_sources_message(
                format!("{} {e}", t("Captura indisponível:", "Capture unavailable:")).into(),
            );
            Vec::new()
        }
    };
    // Jogos prováveis primeiro, depois janelas, depois monitores (S-003).
    list.sort_by_key(|s| {
        (
            !s.likely_game,
            s.kind == SourceKind::Monitor,
            s.title.to_lowercase(),
        )
    });
    let items: Vec<SourceItem> = list
        .iter()
        .map(|s| SourceItem {
            title: s.title.clone().into(),
            detail: source_detail(s).into(),
            game: s.likely_game,
        })
        .collect();
    if items.is_empty() && ui.get_sources_message().is_empty() {
        ui.set_sources_message(t(
            "Nenhuma janela ou monitor encontrado. Abra o jogo e toque em Atualizar lista.",
            "No window or monitor found. Open the game and select Refresh list.",
        ));
    }
    let selected = previous
        .and_then(|id| list.iter().position(|s| s.id == id))
        .map_or(-1, |i| i as i32);
    ui.set_sources(ModelRc::new(VecModel::from(items)));
    ui.set_selected_source(selected);
    with_state(|st| st.sources = list);
}

// ---------------------------------------------------------------------------
// Mensagens do motor

fn on_engine_msg(ui: &AppWindow, msg: EngineMsg) {
    match msg {
        EngineMsg::Sources(result) => set_sources(ui, result),
        EngineMsg::Encoders(result) => {
            let list = result.unwrap_or_default();
            with_state(|st| st.encoders = list.clone());
            set_encoder_options(ui, &list);
        }
        EngineMsg::StartFailed(e) => {
            with_state(|st| st.live = None);
            ui.set_starting(false);
            set_stream_message(
                ui,
                &format!("{} {e}", t("Não foi possível iniciar:", "Could not start:")),
                true,
            );
            sync_live(ui);
        }
        EngineMsg::Event(event) => on_session_event(ui, event),
        EngineMsg::Stats(stats) => show_stats(ui, &stats),
    }
}

fn on_session_event(ui: &AppWindow, event: SessionEvent) {
    let active = with_state(|st| st.live.is_some()).unwrap_or(false);
    match event {
        SessionEvent::Started {
            encoder,
            width,
            height,
            fps,
        } => {
            if !active {
                return;
            }
            with_state(|st| {
                if let Some(l) = &mut st.live {
                    l.started = Some(Instant::now());
                }
            });
            ui.set_starting(false);
            ui.set_live_video(format!("{width}×{height} @ {fps} · {encoder}").into());
            ui.set_live_status(t("Transmitindo", "Streaming"));
            set_stream_message(ui, "", false);
            sync_live(ui);
            crate::navigate_to(ui, LIVE_PAGE);
        }
        SessionEvent::Connected => {
            if active {
                ui.set_live_status(t(
                    "Transmitindo · conectado ao servidor",
                    "Streaming · connected to the server",
                ));
                set_live_message(ui, "", false);
            }
        }
        SessionEvent::Reconnecting { reason } => {
            if active {
                ui.set_live_status(t(
                    "Reconectando ao servidor…",
                    "Reconnecting to the server…",
                ));
                tracing::info!(%reason, "reconectando ao relay");
            }
        }
        SessionEvent::Warning { message } => {
            if active {
                set_live_message(ui, &message, false);
            }
        }
        SessionEvent::SourceLost => {
            if active {
                let msg = t(
                    "A janela ou o monitor capturado não está mais disponível. A transmissão foi encerrada.",
                    "The captured window or monitor is no longer available. The stream was stopped.",
                );
                stop_stream(ui, Some(msg.as_str()));
            }
        }
        SessionEvent::Stopped { reason } => {
            if active {
                let msg = reason
                    .map(|r| format!("{} {r}", t("Transmissão encerrada:", "Stream stopped:")));
                end_live(ui, msg.as_deref());
            }
        }
    }
}

fn show_stats(ui: &AppWindow, s: &SessionStats) {
    let dash = || SharedString::from("—");
    ui.set_live_captured_fps(format!("{:.0}", s.captured_fps).into());
    ui.set_live_encoded_fps(format!("{:.0}", s.encoded_fps).into());
    ui.set_live_bitrate(format!("{:.0} kbps", s.bitrate_kbps).into());
    ui.set_live_dropped(format!("{} / {}", s.frames_dropped, s.network_dropped).into());
    ui.set_live_encode_ms(if s.encode_ms_avg > 0.0 {
        format!("{:.1} ms", s.encode_ms_avg).into()
    } else {
        dash()
    });
    ui.set_live_rtt(s.rtt_ms.map_or_else(dash, |r| format!("{r:.0} ms").into()));
    ui.set_live_target(if s.target_bitrate_kbps > 0 {
        format!("{} kbps · {} FPS", s.target_bitrate_kbps, s.target_fps).into()
    } else {
        dash()
    });
    // CPU e memória do app: amostrados no mesmo ritmo, só com a tela Ao vivo visível.
    let proc = PROCESS_SAMPLER.with(|p| p.borrow_mut().sample());
    ui.set_live_app_cpu(
        proc.cpu_percent
            .map_or_else(dash, |c| format!("{c:.1}%").into()),
    );
    ui.set_live_app_ram(
        proc.memory_mb
            .map_or_else(dash, |m| format!("{m:.0} MB").into()),
    );
}

thread_local! {
    static PROCESS_SAMPLER: std::cell::RefCell<crate::procstats::ProcessSampler> =
        std::cell::RefCell::new(crate::procstats::ProcessSampler::default());
}

// ---------------------------------------------------------------------------
// Transmissão

fn set_stream_message(ui: &AppWindow, text: &str, error: bool) {
    ui.set_stream_message(text.into());
    ui.set_stream_message_error(error);
}

fn set_live_message(ui: &AppWindow, text: &str, error: bool) {
    ui.set_live_message(text.into());
    ui.set_live_message_error(error);
}

fn start_stream(ui: &AppWindow) {
    if with_state(|st| st.live.is_some()).unwrap_or(true) {
        return;
    }
    let source = with_state(|st| {
        let i = ui.get_selected_source();
        (i >= 0)
            .then(|| st.sources.get(i as usize).cloned())
            .flatten()
    })
    .flatten();
    let room = with_state(|st| {
        st.rooms
            .rooms
            .get(ui.get_stream_room_index().max(0) as usize)
            .cloned()
    })
    .flatten();
    let (Some(source), Some(room)) = (source, room) else {
        set_stream_message(
            ui,
            &t(
                "Escolha o que compartilhar e a sala de destino.",
                "Choose what to share and the destination room.",
            ),
            true,
        );
        return;
    };
    let Some((prefs, encoders)) =
        with_state(|st| (st.settings.stream.clone(), st.encoders.clone()))
    else {
        return;
    };
    let ws_url = match relay_api::ws_url(&room.relay) {
        Ok(u) => u,
        Err(e) => {
            set_stream_message(ui, e.message(en()), true);
            return;
        }
    };
    let mut audio = prefs.audio_mode();
    if source.kind == SourceKind::Monitor && audio == AudioMode::Application {
        // Monitor não tem processo dono: sem áudio, em vez de capturar o sistema todo sem aviso.
        audio = AudioMode::Off;
    }
    let encoder = if prefs.encoder.is_empty() || !encoders.iter().any(|e| e.name == prefs.encoder) {
        EncoderPreference::Auto
    } else {
        EncoderPreference::Named(prefs.encoder.clone())
    };

    ui.set_starting(true);
    set_stream_message(ui, &t("Preparando a sala…", "Preparing the room…"), false);
    with_state(|st| {
        st.live = Some(LiveSession {
            room_id: room.id.clone(),
            relay: room.relay.clone(),
            owner_token: room.owner_token.clone(),
            started: None,
            ticks: 0,
        });
    });
    ui.set_live_source(source.title.clone().into());
    ui.set_live_room(room.name.clone().into());
    ui.set_live_viewers("0".into());
    ui.set_live_uptime("00:00:00".into());
    set_live_message(ui, "", false);

    let quality = prefs.quality();
    let adaptation = prefs.adaptation();
    let (relay, room_id, owner) = (
        room.relay.clone(),
        room.id.clone(),
        room.owner_token.clone(),
    );
    background(
        ui,
        move || relay_api::publisher_token(&relay, &room_id, &owner),
        move |ui, result| match result {
            Ok(publisher_token) => {
                // O usuário pode ter cancelado enquanto o token era emitido.
                if with_state(|st| st.live.is_none()).unwrap_or(true) {
                    return;
                }
                set_stream_message(ui, &t("Iniciando captura…", "Starting capture…"), false);
                let config = SessionConfig {
                    source,
                    quality,
                    audio,
                    encoder,
                    relay_ws_url: ws_url,
                    publisher_token,
                    adaptation,
                };
                with_state(|st| {
                    if let Some(e) = &st.engine {
                        e.send(Command::Start(Box::new(config)));
                    }
                });
            }
            Err(e) => {
                with_state(|st| st.live = None);
                ui.set_starting(false);
                if e == ApiError::RoomGone {
                    forget_room(ui, &room.id);
                }
                set_stream_message(ui, &e.user_message(en()), true);
            }
        },
    );
}

/// Parada pedida pelo usuário (ou por perda da fonte).
pub fn stop_stream(ui: &AppWindow, message: Option<&str>) {
    with_state(|st| {
        if let Some(e) = &st.engine {
            e.send(Command::Stop);
        }
    });
    end_live(ui, message);
}

fn end_live(ui: &AppWindow, message: Option<&str>) {
    with_state(|st| st.live = None);
    ui.set_starting(false);
    match message {
        Some(m) => {
            set_live_message(ui, m, true);
            set_stream_message(ui, m, true);
        }
        None => {
            set_live_message(ui, &t("Transmissão encerrada.", "Stream stopped."), false);
            set_stream_message(ui, "", false);
        }
    }
    sync_live(ui);
}

/// Propaga o estado "ao vivo" para janela, barra lateral, bandeja e timer.
pub fn sync_live(ui: &AppWindow) {
    let live =
        with_state(|st| st.live.as_ref().is_some_and(|l| l.started.is_some())).unwrap_or(false);
    ui.set_live(live);
    with_state(|st| {
        if let Some(tray) = &st.tray {
            tray.set_live(live, st.settings.language);
        }
    });
    refresh_rooms_ui(ui);
    update_live_timer(ui);
}

/// O timer de métricas só roda com a transmissão ativa, a janela visível e a tela Ao vivo aberta.
pub fn update_live_timer(ui: &AppWindow) {
    let should_run =
        with_state(|st| st.window_visible && st.live.as_ref().is_some_and(|l| l.started.is_some()))
            .unwrap_or(false)
            && ui.get_current_page() == LIVE_PAGE;
    with_state(|st| {
        let Some(timer) = &st.live_timer else { return };
        if should_run && !timer.running() {
            let w = ui.as_weak();
            timer.start(TimerMode::Repeated, TICK, move || {
                if let Some(ui) = w.upgrade() {
                    tick(&ui);
                }
            });
        } else if !should_run && timer.running() {
            timer.stop();
        }
    });
    if should_run {
        tick(ui);
    }
}

fn tick(ui: &AppWindow) {
    let info = with_state(|st| {
        let l = st.live.as_mut()?;
        l.ticks = l.ticks.wrapping_add(1);
        if let Some(e) = &st.engine {
            e.send(Command::Stats);
        }
        Some((
            l.started.map(|s| s.elapsed()),
            l.ticks % VIEWER_POLL_TICKS == 1,
            l.relay.clone(),
            l.room_id.clone(),
            l.owner_token.clone(),
        ))
    })
    .flatten();
    let Some((elapsed, poll, relay, room_id, owner)) = info else {
        return;
    };
    if let Some(e) = elapsed {
        let s = e.as_secs();
        ui.set_live_uptime(format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60).into());
    }
    if poll {
        background(
            ui,
            move || relay_api::get_room(&relay, &room_id, &owner),
            |ui, result| {
                if let Ok(room) = result {
                    ui.set_live_viewers(format!("{} / {}", room.viewers, room.max_viewers).into());
                }
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Salas e convites

fn set_rooms_message(ui: &AppWindow, text: &str, error: bool) {
    ui.set_rooms_message(text.into());
    ui.set_rooms_message_error(error);
}

pub fn refresh_rooms_ui(ui: &AppWindow) {
    let now = now_unix();
    let Some((rooms, live_room)) = with_state(|st| {
        st.rooms.prune(now);
        (
            st.rooms.rooms.clone(),
            st.live
                .as_ref()
                .filter(|l| l.started.is_some())
                .map(|l| l.room_id.clone()),
        )
    }) else {
        return;
    };
    let items: Vec<RoomItem> = rooms
        .iter()
        .map(|r| {
            let hours = r.expires_at.saturating_sub(now) / 3600;
            let detail = if en() {
                format!("{} · expires in {hours} h", r.relay)
            } else {
                format!("{} · expira em {hours} h", r.relay)
            };
            RoomItem {
                name: r.name.clone().into(),
                detail: detail.into(),
                live: live_room.as_deref() == Some(r.id.as_str()),
            }
        })
        .collect();
    let names: Vec<SharedString> = rooms.iter().map(|r| r.name.clone().into()).collect();
    let count = rooms.len() as i32;
    ui.set_rooms(ModelRc::new(VecModel::from(items)));
    ui.set_room_names(ModelRc::new(VecModel::from(names)));
    // Seleciona a primeira sala por padrão para as ações ficarem visíveis.
    if ui.get_rooms_index() >= count || (ui.get_rooms_index() < 0 && count > 0) {
        ui.set_rooms_index(if count > 0 { 0 } else { -1 });
    }
    if ui.get_stream_room_index() >= count || ui.get_stream_room_index() < 0 {
        ui.set_stream_room_index(0);
    }
}

fn save_rooms() {
    with_state(|st| {
        if let Some(path) = &st.rooms_path
            && let Err(e) = rooms::save(path, &st.rooms)
        {
            tracing::warn!(%e, "não foi possível salvar as salas");
        }
    });
}

fn forget_room(ui: &AppWindow, id: &str) {
    with_state(|st| st.rooms.remove(id));
    save_rooms();
    ui.set_invite_link(SharedString::new());
    refresh_rooms_ui(ui);
}

fn create_room(ui: &AppWindow, name: &str) {
    let mut name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(64)
        .collect::<String>()
        .trim()
        .to_owned();
    if name.is_empty() {
        name = with_state(|st| st.settings.display_name.clone())
            .filter(|n| !n.is_empty())
            .map_or_else(
                || t("Minha sala", "My room").to_string(),
                |n| format!("{} · Vellcast", n),
            );
    }
    let Some((relay, key)) =
        with_state(|st| (st.settings.relay_url.clone(), st.settings.room_key.clone()))
    else {
        return;
    };
    let relay = match relay_api::normalize_base(&relay) {
        Ok(r) => r,
        Err(e) => {
            set_rooms_message(ui, e.message(en()), true);
            return;
        }
    };
    ui.set_rooms_busy(true);
    set_rooms_message(ui, &t("Criando sala…", "Creating room…"), false);
    background(
        ui,
        {
            let relay = relay.clone();
            move || relay_api::create_room(&relay, &key, &name)
        },
        move |ui, result| {
            ui.set_rooms_busy(false);
            match result {
                Ok(created) => {
                    let room = SavedRoom {
                        id: created.room.id.clone(),
                        name: created.room.name.clone(),
                        relay,
                        owner_token: created.owner_token,
                        created_at: now_unix(),
                        expires_at: created.room.expires_at,
                    };
                    with_state(|st| st.rooms.add(room));
                    save_rooms();
                    ui.set_new_room_name(SharedString::new());
                    refresh_rooms_ui(ui);
                    ui.set_rooms_index(0);
                    ui.set_stream_room_index(0);
                    set_rooms_message(
                        ui,
                        &t(
                            "Sala criada. Gere um link de convite para quem vai assistir.",
                            "Room created. Create an invite link for your viewers.",
                        ),
                        false,
                    );
                }
                Err(e) => set_rooms_message(ui, &e.user_message(en()), true),
            }
        },
    );
}

fn make_invite(ui: &AppWindow, room_id: String) {
    let Some(room) = with_state(|st| st.rooms.get(&room_id).cloned()).flatten() else {
        return;
    };
    let public = with_state(|st| st.settings.public_url.clone()).unwrap_or_default();
    // Links usam o endereço público (túnel/proxy HTTPS) quando configurado.
    let link_base = if public.trim().is_empty() {
        room.relay.clone()
    } else {
        public
    };
    ui.set_rooms_busy(true);
    let (relay, owner) = (room.relay.clone(), room.owner_token.clone());
    let id = room.id.clone();
    background(
        ui,
        move || relay_api::create_invite(&relay, &id, &owner),
        move |ui, result| {
            ui.set_rooms_busy(false);
            match result.and_then(|inv| {
                relay_api::watch_link(&link_base, &room.id, &inv.code).map_err(ApiError::Url)
            }) {
                Ok(link) => {
                    ui.set_invite_link(link.into());
                    let (msg, warn) = if relay_api::is_loopback_base(&link_base) {
                        (
                            t(
                                "Link criado, mas ele só abre neste PC. Para outros aparelhos, configure um endereço público HTTPS em Configurações → Servidor.",
                                "Link created, but it only opens on this PC. For other devices, set a public HTTPS address in Settings → Server.",
                            ),
                            true,
                        )
                    } else if !relay_api::link_plays_in_browser(&link_base) {
                        (
                            t(
                                "Link criado, mas sem HTTPS o navegador não decodifica o vídeo (WebCodecs exige conexão segura). Use um endereço público HTTPS.",
                                "Link created, but without HTTPS browsers cannot decode the video (WebCodecs needs a secure connection). Use a public HTTPS address.",
                            ),
                            true,
                        )
                    } else {
                        (
                            t(
                                "Link de convite criado. Copie e envie para quem vai assistir.",
                                "Invite link created. Copy it and send it to your viewers.",
                            ),
                            false,
                        )
                    };
                    set_rooms_message(ui, &msg, warn);
                    set_live_message(ui, &msg, warn);
                }
                Err(e) => {
                    if e == ApiError::RoomGone {
                        forget_room(ui, &room.id);
                    }
                    let msg = e.user_message(en());
                    set_rooms_message(ui, &msg, true);
                    set_live_message(ui, &msg, true);
                }
            }
        },
    );
}

/// Código para parear a sala com uma instância da Discord Activity (M-015).
fn make_pairing(ui: &AppWindow, room_id: String) {
    let Some(room) = with_state(|st| st.rooms.get(&room_id).cloned()).flatten() else {
        return;
    };
    ui.set_rooms_busy(true);
    let (relay, owner, id) = (
        room.relay.clone(),
        room.owner_token.clone(),
        room.id.clone(),
    );
    background(
        ui,
        move || relay_api::pairing_code(&relay, &id, &owner),
        move |ui, result| {
            ui.set_rooms_busy(false);
            match result {
                Ok(code) => {
                    ui.set_pairing_code(relay_api::format_pairing_code(&code.code).into());
                    set_rooms_message(
                        ui,
                        &t(
                            "Código criado. Na Activity do Vellcast no Discord, digite este código em até 5 minutos.",
                            "Code created. In the Vellcast Activity on Discord, enter this code within 5 minutes.",
                        ),
                        false,
                    );
                }
                Err(e) => {
                    if e == ApiError::RoomGone {
                        forget_room(ui, &room.id);
                    }
                    set_rooms_message(ui, &e.user_message(en()), true);
                }
            }
        },
    );
}

fn copy_invite(ui: &AppWindow) {
    let link = ui.get_invite_link().to_string();
    if link.is_empty() {
        return;
    }
    let result = arboard::Clipboard::new().and_then(|mut c| c.set_text(link));
    let (msg, error) = match result {
        Ok(()) => (
            t(
                "Link copiado para a área de transferência.",
                "Link copied to the clipboard.",
            ),
            false,
        ),
        Err(e) => {
            tracing::warn!(%e, "área de transferência indisponível");
            (
                t(
                    "Não foi possível copiar automaticamente. Selecione o link e copie manualmente.",
                    "Could not copy automatically. Select the link and copy it manually.",
                ),
                true,
            )
        }
    };
    set_rooms_message(ui, &msg, error);
    set_live_message(ui, &msg, error);
}

fn close_room(ui: &AppWindow, index: i32) {
    let Some(room) = with_state(|st| st.rooms.rooms.get(index.max(0) as usize).cloned()).flatten()
    else {
        return;
    };
    let live_here =
        with_state(|st| st.live.as_ref().is_some_and(|l| l.room_id == room.id)).unwrap_or(false);
    if live_here {
        set_rooms_message(
            ui,
            &t(
                "Pare a transmissão antes de encerrar esta sala.",
                "Stop the stream before closing this room.",
            ),
            true,
        );
        return;
    }
    ui.set_rooms_busy(true);
    let (relay, id, owner) = (
        room.relay.clone(),
        room.id.clone(),
        room.owner_token.clone(),
    );
    background(
        ui,
        move || relay_api::delete_room(&relay, &id, &owner),
        move |ui, result| {
            ui.set_rooms_busy(false);
            match result {
                Ok(()) | Err(ApiError::RoomGone) => {
                    forget_room(ui, &room.id);
                    set_rooms_message(
                        ui,
                        &t(
                            "Sala encerrada. Os links de convite dela deixaram de funcionar.",
                            "Room closed. Its invite links no longer work.",
                        ),
                        false,
                    );
                }
                Err(e) => set_rooms_message(ui, &e.user_message(en()), true),
            }
        },
    );
}

fn test_relay(ui: &AppWindow) {
    let Some(relay) = with_state(|st| st.settings.relay_url.clone()) else {
        return;
    };
    ui.set_relay_message(t("Testando…", "Testing…"));
    ui.set_relay_message_error(false);
    background(
        ui,
        move || {
            crate::local_relay::ensure(&relay);
            relay_api::health(&relay)
        },
        |ui, result| match result {
            Ok(()) => {
                ui.set_relay_message(t(
                    "Servidor respondeu. Tudo certo.",
                    "The server responded. All good.",
                ));
                ui.set_relay_message_error(false);
            }
            Err(e) => {
                ui.set_relay_message(e.user_message(en()).into());
                ui.set_relay_message_error(true);
            }
        },
    );
}

/// Reaplica os textos gerados em Rust após a troca de idioma.
pub fn on_language_changed(ui: &AppWindow) {
    update_quality_summary(ui);
    let encoders = with_state(|st| st.encoders.clone()).unwrap_or_default();
    set_encoder_options(ui, &encoders);
    refresh_rooms_ui(ui);
    let sources = with_state(|st| st.sources.clone()).unwrap_or_default();
    if !sources.is_empty() {
        let selected = ui.get_selected_source();
        set_sources(ui, Ok(sources));
        ui.set_selected_source(selected);
    }
}

/// Fluxo automático para o E2E no Windows (feature `dev-snapshot`): cria uma
/// sala, escolhe o primeiro monitor e inicia a transmissão pelo mesmo caminho
/// do botão "Iniciar transmissão". Só roda com `VELLCAST_DEV_AUTOSTREAM=1`.
#[cfg(feature = "dev-snapshot")]
pub fn dev_autostream(ui: &AppWindow) {
    use slint::Timer;
    use std::time::Duration;

    create_room(ui, "E2E Windows");
    let weak = ui.as_weak();
    // Espera a sala ser criada e a lista de fontes carregar.
    Timer::single_shot(Duration::from_secs(5), move || {
        let Some(ui) = weak.upgrade() else { return };
        let monitor = with_state(|st| {
            st.sources
                .iter()
                .position(|s| s.kind == vellcast_core::SourceKind::Monitor)
        })
        .flatten();
        match monitor {
            Some(i) => {
                ui.set_selected_source(i as i32);
                ui.set_stream_room_index(0);
                tracing::info!(source = i, "E2E: iniciando transmissão pelo app");
                start_stream(&ui);
            }
            None => tracing::error!("E2E: nenhum monitor na lista de fontes"),
        }
    });
}
