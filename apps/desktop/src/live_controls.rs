//! Controles rápidos da tela Ao vivo (issue #5): trocar a fonte, ajustar a
//! qualidade e controlar o áudio do jogo e o microfone durante a transmissão.
//!
//! O estado vive na thread da interface (`CONTROLS`) e espelha o global
//! `LiveControlsState` do Slint. As mudanças viram [`LiveChange`] enviadas ao
//! motor ([`Command::Apply`]); nada aqui bloqueia a interface. O que muda ao
//! vivo e o que reinicia captura/encoder vem de `vellcast_core::live`, e as
//! opções de qualidade refletem o pipeline e os limites do plano
//! ([`PlanLimits`]). Sem timers próprios: tudo reage a eventos, e o estado do
//! microfone chega junto com as métricas (que só rodam com a tela Ao vivo aberta).

use std::cell::RefCell;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use vellcast_core::live::{
    self, ContentPreference, FpsOption, PlanLimits, ResolutionOption, Unavailable, VideoSettings,
};
use vellcast_core::{
    Adaptation, AudioInputInfo, AudioMode, LiveChange, MicStatus, MicrophoneConfig, Quality,
    SessionConfig, SessionStats, SourceInfo, SourceKind,
};

use crate::engine::Command;
use crate::settings::Language;
use crate::ui::{AppWindow, LiveControlsState, LiveSourceItem};
use crate::with_state;

const PANEL_SOURCE: i32 = 1;
const PANEL_QUALITY: i32 = 2;
const PANEL_MIC: i32 = 4;

/// Configuração em vigor na transmissão.
#[derive(Debug, Clone)]
struct Current {
    source: SourceInfo,
    quality: Quality,
    adaptation: Adaptation,
    /// Modo do perfil escolhido antes de transmitir (equilibrado ou competitivo).
    motion_mode: Adaptation,
    audio: AudioMode,
    /// Origem lembrada para religar o áudio do jogo.
    game_source: AudioMode,
    /// `None` = microfone desligado (padrão).
    mic: Option<MicrophoneConfig>,
    /// Dispositivo escolhido no painel, mantido entre liga/desliga.
    mic_device: Option<String>,
    live_bitrate: Option<bool>,
    source_lost: bool,
}

impl Current {
    fn video(&self) -> VideoSettings {
        VideoSettings {
            source: self.source.id,
            max_height: self.quality.max_height,
            fps: self.quality.fps,
            bitrate_kbps: self.quality.bitrate_kbps,
            adaptation: self.adaptation,
        }
    }
}

/// Reinício pedido e ainda não confirmado pelo motor (`VideoRestarted`).
#[derive(Debug, Clone)]
struct Pending {
    source: SourceInfo,
    quality: Quality,
    adaptation: Adaptation,
}

#[derive(Default)]
struct Controls {
    current: Option<Current>,
    pending: Option<Pending>,
    /// Fontes do painel, na mesma ordem do modelo do Slint.
    sources: Vec<SourceInfo>,
    inputs: Vec<AudioInputInfo>,
    plan: PlanLimits,
}

thread_local! {
    static CONTROLS: RefCell<Controls> = RefCell::new(Controls::default());
}

fn with_controls<R>(f: impl FnOnce(&mut Controls) -> R) -> R {
    CONTROLS.with(|c| f(&mut c.borrow_mut()))
}

fn en() -> bool {
    with_state(|st| st.settings.language == Language::En).unwrap_or(false)
}

fn t(pt: &str, en_text: &str) -> SharedString {
    if en() { en_text.into() } else { pt.into() }
}

fn send(cmd: Command) {
    with_state(|st| {
        if let Some(e) = &st.engine {
            e.send(cmd);
        }
    });
}

fn state(ui: &AppWindow) -> LiveControlsState<'_> {
    ui.global::<LiveControlsState>()
}

fn set_message(ui: &AppWindow, text: &str, error: bool) {
    let s = state(ui);
    s.set_message(text.into());
    s.set_message_error(error);
}

// ---------------------------------------------------------------------------
// Lógica pura (testada abaixo)

/// Opções utilizáveis (as indisponíveis ficam só na nota explicativa).
fn usable_resolutions(plan: PlanLimits) -> Vec<ResolutionOption> {
    live::resolution_options(plan)
        .into_iter()
        .filter(|o| o.unavailable.is_none())
        .collect()
}

fn usable_fps(plan: PlanLimits) -> Vec<FpsOption> {
    live::fps_options(plan)
        .into_iter()
        .filter(|o| o.unavailable.is_none())
        .collect()
}

/// Índice da maior resolução que não passa de `height`.
fn resolution_index(options: &[ResolutionOption], height: u32) -> usize {
    options
        .iter()
        .rposition(|o| o.height <= height)
        .unwrap_or(0)
}

fn fps_index(options: &[FpsOption], fps: u32) -> usize {
    options.iter().rposition(|o| o.fps <= fps).unwrap_or(0)
}

/// Nota sobre as opções que existem, mas não podem ser escolhidas.
fn unavailable_note(plan: PlanLimits, english: bool) -> String {
    let mut by_plan = Vec::new();
    let mut by_engine = Vec::new();
    for o in live::resolution_options(plan) {
        match o.unavailable {
            Some(Unavailable::Plan) => by_plan.push(format!("{}p", o.height)),
            Some(Unavailable::Engine) => by_engine.push(format!("{}p", o.height)),
            None => {}
        }
    }
    for o in live::fps_options(plan) {
        match o.unavailable {
            Some(Unavailable::Plan) => by_plan.push(format!("{} FPS", o.fps)),
            Some(Unavailable::Engine) => by_engine.push(format!("{} FPS", o.fps)),
            None => {}
        }
    }
    let free = plan == PlanLimits::FREE;
    let mut parts = Vec::new();
    if !by_plan.is_empty() {
        let list = by_plan.join(", ");
        parts.push(match (english, free) {
            (true, true) => format!("{list}: not included in the Free plan."),
            (true, false) => format!("{list}: not included in your plan."),
            (false, true) => format!("{list}: não incluídos no plano Gratuito."),
            (false, false) => format!("{list}: não incluídos no seu plano."),
        });
    }
    if !by_engine.is_empty() {
        let list = by_engine.join(", ");
        parts.push(if english {
            format!("{list}: not supported by Vellcast yet.")
        } else {
            format!("{list}: ainda não suportados pelo Vellcast.")
        });
    }
    parts.join(" ")
}

/// Rótulo do botão de qualidade, ex.: "1080p · 60 FPS · 8 Mbps".
fn quality_label(q: &Quality, english: bool) -> String {
    let mbps = f64::from(q.bitrate_kbps) / 1000.0;
    let mbps = if q.bitrate_kbps.is_multiple_of(1000) {
        format!("{mbps:.0}")
    } else if english {
        format!("{mbps:.1}")
    } else {
        format!("{mbps:.1}").replace('.', ",")
    };
    format!("{}p · {} FPS · {mbps} Mbps", q.max_height, q.fps)
}

/// Modo do áudio do jogo a partir do painel. Em monitores só existe "sistema".
fn audio_mode_from_panel(enabled: bool, index: i32, monitor: bool) -> AudioMode {
    match (enabled, monitor, index) {
        (false, _, _) => AudioMode::Off,
        (true, true, _) | (true, false, 1) => AudioMode::System,
        (true, false, _) => AudioMode::Application,
    }
}

/// Índice do seletor de origem para um modo (inverso de [`audio_mode_from_panel`]).
fn audio_index(mode: AudioMode, monitor: bool) -> i32 {
    i32::from(!monitor && mode == AudioMode::System)
}

/// Opções do seletor de microfone: o padrão do Windows primeiro.
fn mic_options(inputs: &[AudioInputInfo], english: bool) -> Vec<String> {
    let default_name = inputs
        .iter()
        .find(|d| d.is_default)
        .map(|d| d.name.as_str());
    let mut out = vec![match (default_name, english) {
        (Some(n), true) => format!("Windows default ({n})"),
        (Some(n), false) => format!("Padrão do Windows ({n})"),
        (None, true) => "Windows default".to_string(),
        (None, false) => "Padrão do Windows".to_string(),
    }];
    out.extend(inputs.iter().map(|d| d.name.clone()));
    out
}

/// Dispositivo do índice escolhido (0 = padrão do Windows).
fn mic_device_at(inputs: &[AudioInputInfo], index: i32) -> Option<String> {
    (index > 0)
        .then(|| inputs.get(index as usize - 1).map(|d| d.id.clone()))
        .flatten()
}

fn mic_index_of(inputs: &[AudioInputInfo], device: Option<&str>) -> i32 {
    device
        .and_then(|id| inputs.iter().position(|d| d.id == id))
        .map_or(0, |i| i as i32 + 1)
}

fn mic_state_index(status: MicStatus) -> i32 {
    match status {
        MicStatus::Off => 0,
        MicStatus::Starting => 1,
        MicStatus::Active => 2,
        MicStatus::Muted => 3,
        MicStatus::Unavailable => 4,
    }
}

fn source_detail(s: &SourceInfo, english: bool) -> String {
    let kind = match (s.kind, english) {
        (SourceKind::Window, true) => "Window",
        (SourceKind::Window, false) => "Janela",
        (SourceKind::Monitor, _) => "Monitor",
    };
    match &s.process_name {
        Some(p) => format!("{kind} · {p} · {}×{}", s.width, s.height),
        None => format!("{kind} · {}×{}", s.width, s.height),
    }
}

// ---------------------------------------------------------------------------
// Ciclo de vida

pub fn init(ui: &AppWindow) {
    let s = state(ui);
    let (min, max) = live::BITRATE_RANGE_KBPS;
    s.set_bitrate_min(min as f32);
    s.set_bitrate_max(max as f32);

    let w = ui.as_weak();
    s.on_panel_opened(move |panel| {
        if let Some(ui) = w.upgrade() {
            panel_opened(&ui, panel);
        }
    });
    let w = ui.as_weak();
    s.on_refresh_sources(move || {
        if let Some(ui) = w.upgrade() {
            state(&ui).set_sources_loading(true);
            send(Command::ListSources);
        }
    });
    let w = ui.as_weak();
    s.on_source_picked(move |i| {
        if let Some(ui) = w.upgrade() {
            source_picked(&ui, i);
        }
    });
    let w = ui.as_weak();
    s.on_quality_edited(move || {
        if let Some(ui) = w.upgrade() {
            quality_edited(&ui);
        }
    });
    let w = ui.as_weak();
    s.on_apply_quality(move || {
        if let Some(ui) = w.upgrade() {
            apply_quality(&ui);
        }
    });
    let w = ui.as_weak();
    s.on_confirm_restart(move || {
        if let Some(ui) = w.upgrade() {
            confirm_restart(&ui);
        }
    });
    let w = ui.as_weak();
    s.on_cancel_changes(move || {
        if let Some(ui) = w.upgrade() {
            push_all(&ui);
        }
    });
    let w = ui.as_weak();
    s.on_game_audio_changed(move || {
        if let Some(ui) = w.upgrade() {
            game_audio_changed(&ui);
        }
    });
    let w = ui.as_weak();
    s.on_mic_changed(move || {
        if let Some(ui) = w.upgrade() {
            mic_changed(&ui);
        }
    });
    let w = ui.as_weak();
    s.on_mic_mute_changed(move || {
        if let Some(ui) = w.upgrade() {
            let muted = state(&ui).get_mic_muted();
            with_controls(|c| {
                if let Some(cur) = &mut c.current
                    && let Some(mic) = &mut cur.mic
                {
                    mic.muted = muted;
                }
            });
            send(Command::Apply(LiveChange::MicrophoneMuted(muted)));
            if state(&ui).get_mic_enabled() {
                state(&ui).set_mic_state(if muted { 3 } else { 2 });
            }
        }
    });
    let w = ui.as_weak();
    s.on_refresh_mics(move || {
        if w.upgrade().is_some() {
            send(Command::ListAudioInputs);
        }
    });
}

/// Nova transmissão: os controles partem da configuração usada no início.
/// Microfone sempre desligado até o usuário ligar (opt-in).
pub fn begin(ui: &AppWindow, config: &SessionConfig) {
    with_controls(|c| {
        c.pending = None;
        c.current = Some(Current {
            source: config.source.clone(),
            quality: config.quality,
            adaptation: config.adaptation,
            motion_mode: config.adaptation,
            audio: config.audio,
            game_source: if config.audio == AudioMode::Off {
                AudioMode::Application
            } else {
                config.audio
            },
            mic: None,
            mic_device: None,
            live_bitrate: None,
            source_lost: false,
        });
    });
    let s = state(ui);
    s.set_open_panel(0);
    s.set_confirming(false);
    s.set_busy(false);
    s.set_mic_state(0);
    set_message(ui, "", false);
    push_all(ui);
}

/// A transmissão terminou: fecha os painéis e esquece o estado.
pub fn end(ui: &AppWindow) {
    with_controls(|c| {
        c.current = None;
        c.pending = None;
    });
    let s = state(ui);
    s.set_open_panel(0);
    s.set_confirming(false);
    s.set_busy(false);
    s.set_mic_enabled(false);
    s.set_mic_muted(false);
    s.set_mic_state(0);
    set_message(ui, "", false);
}

/// Botão B do controle: fecha o painel aberto em vez de voltar de tela.
pub fn handle_back(ui: &AppWindow) -> bool {
    let s = state(ui);
    if s.get_open_panel() == 0 {
        return false;
    }
    if s.get_confirming() {
        s.set_confirming(false);
    } else {
        s.invoke_close_panel();
    }
    true
}

fn active() -> bool {
    with_controls(|c| c.current.is_some())
}

/// Reflete todo o estado atual no global do Slint (descarta rascunhos).
fn push_all(ui: &AppWindow) {
    let Some((cur, plan)) = with_controls(|c| c.current.clone().map(|cur| (cur, c.plan))) else {
        return;
    };
    let english = en();
    let s = state(ui);
    let monitor = cur.source.kind == SourceKind::Monitor;
    s.set_source_title(cur.source.title.clone().into());
    s.set_source_lost(cur.source_lost);
    s.set_monitor_source(monitor);
    s.set_quality_label(quality_label(&cur.quality, english).into());

    let resolutions = usable_resolutions(plan);
    let fps = usable_fps(plan);
    let res_labels: Vec<SharedString> = resolutions
        .iter()
        .map(|o| format!("{}×{}", o.width, o.height).into())
        .collect();
    let fps_labels: Vec<SharedString> = fps
        .iter()
        .map(|o| format!("{} FPS", o.fps).into())
        .collect();
    s.set_resolution_options(ModelRc::new(VecModel::from(res_labels)));
    s.set_fps_options(ModelRc::new(VecModel::from(fps_labels)));
    s.set_resolution_index(resolution_index(&resolutions, cur.quality.max_height) as i32);
    s.set_fps_index(fps_index(&fps, cur.quality.fps) as i32);
    s.set_bitrate(live::snap_bitrate(cur.quality.bitrate_kbps) as f32);
    s.set_content_index(match ContentPreference::from_adaptation(cur.adaptation) {
        ContentPreference::Motion => 0,
        ContentPreference::Clarity => 1,
    });
    s.set_unavailable_note(unavailable_note(plan, english).into());
    s.set_live_bitrate(cur.live_bitrate != Some(false));
    s.set_quality_dirty(false);
    s.set_quality_restart(false);

    s.set_game_audio(cur.audio != AudioMode::Off);
    s.set_game_audio_index(audio_index(
        if cur.audio == AudioMode::Off {
            cur.game_source
        } else {
            cur.audio
        },
        monitor,
    ));

    s.set_mic_enabled(cur.mic.is_some());
    s.set_mic_muted(cur.mic.as_ref().is_some_and(|m| m.muted));
    push_sources(ui);
    push_mics(ui);
}

/// Modelo do painel de fontes a partir da lista do motor (`UiState::sources`).
fn push_sources(ui: &AppWindow) {
    let Some(current) = with_controls(|c| c.current.as_ref().map(|cur| cur.source.id)) else {
        return;
    };
    let mut list = with_state(|st| st.sources.clone()).unwrap_or_default();
    // Mesma ordem da tela Transmitir: jogos, janelas, monitores.
    list.sort_by_key(|s| {
        (
            !s.likely_game,
            s.kind == SourceKind::Monitor,
            s.title.to_lowercase(),
        )
    });
    let english = en();
    let items: Vec<LiveSourceItem> = list
        .iter()
        .map(|s| LiveSourceItem {
            title: s.title.clone().into(),
            detail: source_detail(s, english).into(),
            current: s.id == current,
        })
        .collect();
    let index = list
        .iter()
        .position(|s| s.id == current)
        .map_or(-1, |i| i as i32);
    let s = state(ui);
    s.set_sources(ModelRc::new(VecModel::from(items)));
    s.set_source_index(index);
    s.set_source_dirty(false);
    with_controls(|c| c.sources = list);
}

fn push_mics(ui: &AppWindow) {
    let (inputs, device) = with_controls(|c| {
        (
            c.inputs.clone(),
            c.current.as_ref().and_then(|cur| cur.mic_device.clone()),
        )
    });
    let labels: Vec<SharedString> = mic_options(&inputs, en())
        .into_iter()
        .map(SharedString::from)
        .collect();
    let s = state(ui);
    s.set_mic_options(ModelRc::new(VecModel::from(labels)));
    s.set_mic_index(mic_index_of(&inputs, device.as_deref()));
}

// ---------------------------------------------------------------------------
// Eventos do motor

/// A lista de fontes foi atualizada (`EngineMsg::Sources`).
pub fn sources_updated(ui: &AppWindow) {
    state(ui).set_sources_loading(false);
    if active() {
        push_sources(ui);
    }
}

pub fn set_audio_inputs(ui: &AppWindow, result: Result<Vec<AudioInputInfo>, String>) {
    let s = state(ui);
    match result {
        Ok(list) => {
            s.set_mic_none_found(list.is_empty());
            with_controls(|c| c.inputs = list);
        }
        Err(e) => {
            tracing::warn!(%e, "lista de microfones indisponível");
            s.set_mic_none_found(false);
            if s.get_open_panel() == PANEL_MIC {
                set_message(
                    ui,
                    &format!(
                        "{} {e}",
                        t(
                            "Não foi possível listar os microfones:",
                            "Could not list the microphones:"
                        )
                    ),
                    true,
                );
            }
        }
    }
    push_mics(ui);
}

/// Métricas periódicas: estado do microfone e se o bitrate muda ao vivo.
pub fn on_stats(ui: &AppWindow, stats: &SessionStats) {
    let changed = with_controls(|c| {
        let Some(cur) = &mut c.current else {
            return false;
        };
        let changed = cur.live_bitrate != stats.live_bitrate;
        cur.live_bitrate = stats.live_bitrate;
        changed
    });
    let s = state(ui);
    if s.get_mic_enabled() {
        s.set_mic_state(mic_state_index(stats.microphone));
    }
    if changed {
        s.set_live_bitrate(stats.live_bitrate != Some(false));
        if s.get_open_panel() == PANEL_QUALITY {
            quality_edited(ui);
        }
    }
}

/// A janela ou o monitor sumiu: a sessão continua e o painel de fontes abre.
pub fn on_source_lost(ui: &AppWindow) {
    with_controls(|c| {
        if let Some(cur) = &mut c.current {
            cur.source_lost = true;
        }
    });
    let s = state(ui);
    s.set_source_lost(true);
    s.set_confirming(false);
    s.set_open_panel(PANEL_SOURCE);
    s.set_sources_loading(true);
    send(Command::ListSources);
    set_message(
        ui,
        &t(
            "A fonte foi fechada. Escolha outra para continuar transmitindo.",
            "The source was closed. Choose another one to keep streaming.",
        ),
        true,
    );
}

/// Captura e encoder reiniciados (fonte, resolução ou FPS novos).
pub fn on_video_restarted(ui: &AppWindow, source: &str, width: u32, height: u32, fps: u32) {
    let applied = with_controls(|c| {
        let pending = c.pending.take();
        let cur = c.current.as_mut()?;
        // Sem pedido pendente: reinício do próprio motor (ex.: encoder que não
        // muda o bitrate ao vivo); a configuração já é a atual.
        let ok = pending.as_ref().is_none_or(|p| {
            p.source.title == source
                && p.quality.fps == fps
                && width <= p.quality.max_width
                && height <= p.quality.max_height
        });
        if let Some(p) = pending.filter(|_| ok) {
            let was_monitor = cur.source.kind == SourceKind::Monitor;
            cur.source = p.source;
            cur.quality = p.quality;
            cur.adaptation = p.adaptation;
            cur.source_lost = false;
            // Monitor não tem aplicativo: "só o aplicativo" fica sem áudio (o motor avisa).
            if !was_monitor
                && cur.source.kind == SourceKind::Monitor
                && cur.audio == AudioMode::Application
            {
                cur.audio = AudioMode::Off;
            }
        }
        Some(ok)
    });
    let Some(ok) = applied else { return };
    state(ui).set_busy(false);
    state(ui).set_confirming(false);
    push_all(ui);
    if ok {
        let text = if en() {
            format!("Applied: {width}×{height} at {fps} FPS.")
        } else {
            format!("Aplicado: {width}×{height} a {fps} FPS.")
        };
        set_message(ui, &text, false);
        let panel = state(ui).get_open_panel();
        if panel == PANEL_SOURCE || panel == PANEL_QUALITY {
            state(ui).invoke_close_panel();
        }
    } else {
        set_message(
            ui,
            &t(
                "A mudança não pôde ser aplicada; a configuração anterior continua.",
                "The change could not be applied; the previous settings stay in place.",
            ),
            true,
        );
    }
}

/// Aviso do motor: libera os botões se um reinício estava em andamento.
pub fn on_warning(ui: &AppWindow) {
    let s = state(ui);
    if s.get_busy() && with_controls(|c| c.current.as_ref().is_some_and(|cur| cur.source_lost)) {
        // Sem fonte, um reinício que falha não gera `VideoRestarted`.
        with_controls(|c| c.pending = None);
        s.set_busy(false);
    }
}

// ---------------------------------------------------------------------------
// Ações dos painéis

fn panel_opened(ui: &AppWindow, panel: i32) {
    set_message(ui, "", false);
    push_all(ui);
    match panel {
        PANEL_SOURCE => {
            state(ui).set_sources_loading(true);
            send(Command::ListSources);
        }
        PANEL_MIC => send(Command::ListAudioInputs),
        _ => {}
    }
}

fn source_picked(ui: &AppWindow, index: i32) {
    let s = state(ui);
    s.set_source_index(index);
    let dirty = with_controls(|c| {
        let picked = c.sources.get(index.max(0) as usize)?;
        let cur = c.current.as_ref()?;
        Some(picked.id != cur.source.id || cur.source_lost)
    })
    .unwrap_or(false);
    s.set_source_dirty(dirty);
}

/// Qualidade escolhida no painel (rascunho).
fn draft_quality(ui: &AppWindow) -> Option<(Quality, Adaptation)> {
    let s = state(ui);
    let (plan, cur) = with_controls(|c| c.current.clone().map(|cur| (c.plan, cur)))?;
    let resolutions = usable_resolutions(plan);
    let fps = usable_fps(plan);
    let res = resolutions.get(s.get_resolution_index().max(0) as usize)?;
    let fps = fps.get(s.get_fps_index().max(0) as usize)?;
    let preference = if s.get_content_index() == 1 {
        ContentPreference::Clarity
    } else {
        ContentPreference::Motion
    };
    let quality = Quality {
        max_width: res.width,
        max_height: res.height,
        fps: fps.fps,
        bitrate_kbps: live::snap_bitrate(s.get_bitrate().round().max(0.0) as u32),
    };
    Some((quality, preference.adaptation(cur.motion_mode)))
}

fn plan_for(ui: &AppWindow) -> Option<(live::ChangePlan, Quality, Adaptation, Current)> {
    let (quality, adaptation) = draft_quality(ui)?;
    let cur = with_controls(|c| c.current.clone())?;
    let next = VideoSettings {
        source: cur.source.id,
        max_height: quality.max_height,
        fps: quality.fps,
        bitrate_kbps: quality.bitrate_kbps,
        adaptation,
    };
    let plan = live::plan_video_change(&cur.video(), &next, cur.live_bitrate != Some(false));
    Some((plan, quality, adaptation, cur))
}

fn quality_edited(ui: &AppWindow) {
    let Some((plan, ..)) = plan_for(ui) else {
        return;
    };
    let s = state(ui);
    s.set_quality_dirty(!plan.is_empty());
    s.set_quality_restart(plan.needs_restart());
}

/// Aplica ao vivo (bitrate e preferência de conteúdo), sem interromper a imagem.
fn apply_quality(ui: &AppWindow) {
    let Some((plan, quality, adaptation, _)) = plan_for(ui) else {
        return;
    };
    if plan.needs_restart() {
        state(ui).set_confirming(true);
        return;
    }
    if let Some(mode) = plan.live_adaptation {
        send(Command::Apply(LiveChange::Adaptation(mode)));
    }
    if let Some(kbps) = plan.live_bitrate {
        send(Command::Apply(LiveChange::Bitrate(kbps)));
    }
    with_controls(|c| {
        if let Some(cur) = &mut c.current {
            cur.quality.bitrate_kbps = quality.bitrate_kbps;
            cur.adaptation = adaptation;
        }
    });
    push_all(ui);
    set_message(
        ui,
        &t(
            "Aplicado sem interromper a transmissão.",
            "Applied without interrupting the stream.",
        ),
        false,
    );
    // Fecha o painel: o foco volta ao botão Qualidade e a mensagem fica visível.
    state(ui).invoke_close_panel();
}

/// Confirmação de reinício: troca de fonte ou qualidade que reinicia o vídeo.
fn confirm_restart(ui: &AppWindow) {
    let s = state(ui);
    let request = match s.get_open_panel() {
        PANEL_SOURCE => {
            let picked = with_controls(|c| {
                let i = s.get_source_index();
                (i >= 0)
                    .then(|| c.sources.get(i as usize).cloned())
                    .flatten()
            });
            let cur = with_controls(|c| c.current.clone());
            picked.zip(cur).map(|(source, cur)| Pending {
                source,
                quality: cur.quality,
                adaptation: cur.adaptation,
            })
        }
        PANEL_QUALITY => plan_for(ui).map(|(_, quality, adaptation, cur)| Pending {
            source: cur.source,
            quality,
            adaptation,
        }),
        _ => None,
    };
    let Some(pending) = request else {
        return;
    };
    let current_adaptation = with_controls(|c| c.current.as_ref().map(|cur| cur.adaptation));
    if current_adaptation != Some(pending.adaptation) {
        send(Command::Apply(LiveChange::Adaptation(pending.adaptation)));
    }
    send(Command::Apply(LiveChange::Video {
        source: pending.source.clone(),
        quality: pending.quality,
    }));
    with_controls(|c| c.pending = Some(pending));
    s.set_busy(true);
    set_message(
        ui,
        &t(
            "Reiniciando captura e encoder…",
            "Restarting capture and the encoder…",
        ),
        false,
    );
}

fn game_audio_changed(ui: &AppWindow) {
    let s = state(ui);
    let Some(monitor) = with_controls(|c| {
        c.current
            .as_ref()
            .map(|cur| cur.source.kind == SourceKind::Monitor)
    }) else {
        return;
    };
    let mode = audio_mode_from_panel(s.get_game_audio(), s.get_game_audio_index(), monitor);
    with_controls(|c| {
        if let Some(cur) = &mut c.current {
            cur.audio = mode;
            if mode != AudioMode::Off {
                cur.game_source = mode;
            }
        }
    });
    send(Command::Apply(LiveChange::Audio(mode)));
    let text = match mode {
        AudioMode::Off => t("Áudio do jogo desligado.", "Game audio turned off."),
        AudioMode::Application => t(
            "Compartilhando só o áudio do aplicativo.",
            "Sharing only the app audio.",
        ),
        AudioMode::System => t(
            "Compartilhando o áudio do sistema (sem a voz do Discord quando possível).",
            "Sharing the system audio (without Discord voice when possible).",
        ),
    };
    set_message(ui, &text, false);
}

fn mic_changed(ui: &AppWindow) {
    let s = state(ui);
    let enabled = s.get_mic_enabled();
    let index = s.get_mic_index();
    let muted = s.get_mic_muted();
    let Some(mic) = with_controls(|c| {
        let device = mic_device_at(&c.inputs, index);
        let cur = c.current.as_mut()?;
        cur.mic_device = device.clone();
        cur.mic = enabled.then_some(MicrophoneConfig {
            device_id: device,
            muted,
        });
        Some(cur.mic.clone())
    }) else {
        return;
    };
    s.set_mic_state(if mic.is_some() { 1 } else { 0 });
    if mic.is_none() {
        s.set_mic_muted(false);
    }
    send(Command::Apply(LiveChange::Microphone(mic)));
}

/// Estado de demonstração para capturas de tela (feature `dev-snapshot`).
#[cfg(feature = "dev-snapshot")]
pub fn demo(ui: &AppWindow) {
    use vellcast_core::SourceId;
    let source = SourceInfo {
        id: SourceId(1),
        kind: SourceKind::Window,
        title: "Jogo de exemplo".into(),
        process_name: Some("jogo.exe".into()),
        process_id: Some(1234),
        width: 1920,
        height: 1080,
        likely_game: true,
    };
    let config = SessionConfig {
        source,
        quality: vellcast_core::QualityProfile::Equilibrado.quality(),
        audio: AudioMode::Application,
        encoder: vellcast_core::EncoderPreference::Auto,
        relay_ws_url: String::new(),
        publisher_token: String::new(),
        adaptation: Adaptation::Balanced,
        display_name: None,
    };
    begin(ui, &config);
    with_controls(|c| {
        c.inputs = vec![AudioInputInfo {
            id: "mic-1".into(),
            name: "Headset USB".into(),
            is_default: true,
        }];
    });
    push_mics(ui);
    if let Ok(p) = std::env::var("VELLCAST_DEV_LIVE_PANEL")
        && let Ok(panel) = p.parse::<i32>()
    {
        state(ui).set_open_panel(panel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str, name: &str, default: bool) -> AudioInputInfo {
        AudioInputInfo {
            id: id.into(),
            name: name.into(),
            is_default: default,
        }
    }

    #[test]
    fn free_plan_offers_up_to_1080p60() {
        let res = usable_resolutions(PlanLimits::FREE);
        assert_eq!(
            res.iter().map(|o| o.height).collect::<Vec<_>>(),
            vec![720, 900, 1080]
        );
        let fps = usable_fps(PlanLimits::FREE);
        assert_eq!(fps.iter().map(|o| o.fps).collect::<Vec<_>>(), vec![30, 60]);
        assert_eq!(resolution_index(&res, 1080), 2);
        assert_eq!(resolution_index(&res, 1000), 1);
        assert_eq!(resolution_index(&res, 480), 0);
        assert_eq!(fps_index(&fps, 60), 1);
        assert_eq!(fps_index(&fps, 45), 0);
    }

    #[test]
    fn unavailable_note_explains_plan_and_engine_limits() {
        assert_eq!(
            unavailable_note(PlanLimits::FREE, true),
            "1440p, 2160p, 120 FPS: not included in the Free plan."
        );
        assert_eq!(
            unavailable_note(PlanLimits::from_entitlement(2160, 120), false),
            "1440p, 2160p, 120 FPS: ainda não suportados pelo Vellcast."
        );
        assert_eq!(
            unavailable_note(PlanLimits::from_entitlement(1440, 60), true),
            "2160p, 120 FPS: not included in your plan. 1440p: not supported by Vellcast yet."
        );
    }

    #[test]
    fn quality_label_is_compact_and_localized() {
        let q = Quality {
            max_width: 1920,
            max_height: 1080,
            fps: 60,
            bitrate_kbps: 8000,
        };
        assert_eq!(quality_label(&q, true), "1080p · 60 FPS · 8 Mbps");
        let q = Quality {
            bitrate_kbps: 2500,
            ..q
        };
        assert_eq!(quality_label(&q, true), "1080p · 60 FPS · 2.5 Mbps");
        assert_eq!(quality_label(&q, false), "1080p · 60 FPS · 2,5 Mbps");
    }

    #[test]
    fn game_audio_panel_maps_to_modes_and_back() {
        assert_eq!(audio_mode_from_panel(false, 1, false), AudioMode::Off);
        assert_eq!(
            audio_mode_from_panel(true, 0, false),
            AudioMode::Application
        );
        assert_eq!(audio_mode_from_panel(true, 1, false), AudioMode::System);
        // Monitor: a única opção é o sistema inteiro.
        assert_eq!(audio_mode_from_panel(true, 0, true), AudioMode::System);
        for monitor in [false, true] {
            for mode in [AudioMode::Application, AudioMode::System] {
                if monitor && mode == AudioMode::Application {
                    continue;
                }
                let i = audio_index(mode, monitor);
                assert_eq!(audio_mode_from_panel(true, i, monitor), mode);
            }
        }
    }

    #[test]
    fn microphone_options_put_windows_default_first() {
        let inputs = vec![
            input("a", "Microfone (USB)", false),
            input("b", "Headset", true),
        ];
        assert_eq!(
            mic_options(&inputs, true),
            vec!["Windows default (Headset)", "Microfone (USB)", "Headset"]
        );
        assert_eq!(mic_options(&[], false), vec!["Padrão do Windows"]);
        assert_eq!(mic_device_at(&inputs, 0), None);
        assert_eq!(mic_device_at(&inputs, 2).as_deref(), Some("b"));
        assert_eq!(mic_device_at(&inputs, 9), None);
        assert_eq!(mic_index_of(&inputs, Some("b")), 2);
        // Dispositivo removido: volta ao padrão.
        assert_eq!(mic_index_of(&inputs, Some("sumiu")), 0);
        assert_eq!(mic_index_of(&inputs, None), 0);
    }

    #[test]
    fn mic_status_indices_match_the_ui() {
        assert_eq!(mic_state_index(MicStatus::Off), 0);
        assert_eq!(mic_state_index(MicStatus::Starting), 1);
        assert_eq!(mic_state_index(MicStatus::Active), 2);
        assert_eq!(mic_state_index(MicStatus::Muted), 3);
        assert_eq!(mic_state_index(MicStatus::Unavailable), 4);
    }
}
