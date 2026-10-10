//! Implementação Windows do pipeline: WGC → conversão NV12 na GPU → H.264
//! (Media Foundation) → relay, com áudio WASAPI/Opus em paralelo.
//!
//! Threads:
//! - `vellcast-pipeline`: dona do dispositivo D3D11, da captura e do encoder;
//!   espera quadros na fila "o mais recente vence" e alimenta o encoder.
//! - pool do WGC (sistema): entrega quadros e aplica o controle de ritmo.
//! - `vellcast-net` (Tokio, 1 worker): conexão com o relay.
//! - thread do `vellcast-audio`: captura e codifica áudio.
//!
//! Nada aqui roda na thread da interface. Todas as filas são limitadas.

mod stats;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use vellcast_capture::clock::{qpc_frequency, qpc_ticks, ticks_to_hns};
use vellcast_capture::slot::LatestSlot;
use vellcast_capture::{
    CaptureError, CaptureOptions, CaptureTarget, CapturedFrame, Capturer, D3dDevice,
};
use vellcast_encoder::sizing::fit_dimensions;
use vellcast_encoder::{
    EncodedFrame, EncoderConfig, EncoderError, H264Encoder, SubmitOutcome, Vendor,
};
use vellcast_protocol::messages::{AudioInit, VideoInit};
use vellcast_transport::{Publisher, PublisherConfig, PublisherEvent, SendOutcome};

use self::stats::{AudioSnapshot, LiveStats, RateWindow};
use super::adapt::{Adapter, Reason, Signals};
use super::*;

/// Intervalo entre keyframes automáticos.
const KEYFRAME_INTERVAL_S: u32 = 2;
/// Tempo máximo para o pipeline inicializar antes de `start` retornar erro.
const INIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Reinícios após perda do dispositivo gráfico antes de desistir.
const MAX_DEVICE_RESETS: u32 = 3;
/// Quadros em codificação rastreados para medir o tempo de codificação.
const MAX_INFLIGHT: usize = 16;
/// Intervalo entre decisões da adaptação de qualidade.
const ADAPT_INTERVAL: Duration = Duration::from_secs(1);
/// Intervalo mínimo entre avisos de redução de qualidade na interface.
const ADAPT_WARNING_INTERVAL: Duration = Duration::from_secs(15);
/// Espera entre voltas do laço enquanto não há fonte (janela fechada).
const WAITING_SOURCE_POLL: Duration = Duration::from_millis(200);

pub fn list_sources() -> Result<Vec<SourceInfo>> {
    let sources = vellcast_capture::list_sources().map_err(map_capture_error)?;
    Ok(sources
        .into_iter()
        .map(|s| {
            let (kind, handle) = match s.target {
                CaptureTarget::Window(h) => (SourceKind::Window, h),
                CaptureTarget::Monitor(h) => (SourceKind::Monitor, h),
            };
            SourceInfo {
                id: SourceId(handle as u64),
                kind,
                title: s.title,
                process_name: s.process_name,
                process_id: s.process_id,
                width: s.width,
                height: s.height,
                likely_game: s.likely_game,
            }
        })
        .collect())
}

pub fn list_encoders() -> Result<Vec<EncoderInfo>> {
    let list = vellcast_encoder::list_encoders().map_err(map_encoder_error)?;
    Ok(list
        .into_iter()
        .map(|e| EncoderInfo {
            name: e.name,
            vendor: match e.vendor {
                Vendor::Nvidia => EncoderVendor::Nvidia,
                Vendor::Amd => EncoderVendor::Amd,
                Vendor::Intel => EncoderVendor::Intel,
                Vendor::Microsoft => EncoderVendor::Microsoft,
                Vendor::Other => EncoderVendor::Other,
            },
            hardware: e.hardware,
        })
        .collect())
}

pub fn list_audio_inputs() -> Result<Vec<AudioInputInfo>> {
    let list = vellcast_audio::list_input_devices().map_err(|e| CoreError::Audio(e.to_string()))?;
    Ok(list
        .into_iter()
        .map(|d| AudioInputInfo {
            id: d.id,
            name: d.name,
            is_default: d.is_default,
        })
        .collect())
}

fn map_capture_error(e: CaptureError) -> CoreError {
    match e {
        CaptureError::Unsupported => CoreError::Unsupported,
        CaptureError::SourceGone => {
            CoreError::SourceUnavailable("a janela ou o monitor não existe mais".into())
        }
        other => CoreError::Capture(other.to_string()),
    }
}

fn map_encoder_error(e: EncoderError) -> CoreError {
    match e {
        EncoderError::NoEncoder => CoreError::NoEncoder,
        EncoderError::Unsupported => CoreError::Unsupported,
        other => CoreError::Encoder(other.to_string()),
    }
}

/// Estado compartilhado entre a sessão (thread da interface) e o pipeline.
struct Shared {
    stop: AtomicBool,
    keyframe: AtomicBool,
    source_lost: AtomicBool,
    fatal: Mutex<Option<String>>,
    slot: Arc<LatestSlot<CapturedFrame>>,
    stats: LiveStats,
}

pub struct Session {
    shared: Arc<Shared>,
    /// Mudanças ao vivo, aplicadas pela thread do pipeline.
    controls: Sender<LiveChange>,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub fn start(config: SessionConfig, events: EventSink) -> Result<Self> {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            keyframe: AtomicBool::new(true),
            source_lost: AtomicBool::new(false),
            fatal: Mutex::new(None),
            slot: Arc::new(LatestSlot::new()),
            stats: LiveStats::new(),
        });
        let (init_tx, init_rx) = sync_channel::<Result<()>>(1);
        let (controls, controls_rx) = channel::<LiveChange>();
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("vellcast-pipeline".into())
            .spawn(move || {
                run(
                    config,
                    thread_shared,
                    Arc::new(events),
                    controls_rx,
                    init_tx,
                )
            })
            .map_err(|e| CoreError::Capture(format!("falha ao criar a thread do pipeline: {e}")))?;

        let session = Self {
            shared,
            controls,
            thread: Some(thread),
        };
        match init_rx.recv_timeout(INIT_TIMEOUT) {
            Ok(Ok(())) => Ok(session),
            Ok(Err(err)) => {
                session.stop();
                Err(err)
            }
            Err(_) => {
                session.stop();
                Err(CoreError::Capture(
                    "o pipeline não inicializou a tempo".into(),
                ))
            }
        }
    }

    pub fn stats(&self) -> SessionStats {
        self.shared.stats.snapshot()
    }

    pub fn request_keyframe(&self) {
        self.shared.keyframe.store(true, Ordering::Relaxed);
    }

    pub fn apply(&self, change: LiveChange) {
        if self.controls.send(change).is_ok() {
            // Acorda o pipeline (pode estar esperando quadro ou sem fonte).
            self.shared.slot.wake();
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.slot.wake();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shutdown();
    }
}

type Events = Arc<EventSink>;

/// Recursos de GPU recriados juntos quando o dispositivo é perdido.
struct Gpu {
    // Ordem de descarte: captura, encoder e por último o dispositivo.
    capturer: Capturer,
    encoder: H264Encoder,
    _device: D3dDevice,
}

fn open_gpu(config: &SessionConfig, shared: &Arc<Shared>, out_w: u32, out_h: u32) -> Result<Gpu> {
    let device = D3dDevice::new().map_err(map_capture_error)?;
    let encoder = H264Encoder::new(
        &device.device,
        EncoderConfig {
            width: out_w,
            height: out_h,
            fps: config.quality.fps,
            bitrate_kbps: config.quality.bitrate_kbps,
            keyframe_interval_s: KEYFRAME_INTERVAL_S,
            preferred: match &config.encoder {
                EncoderPreference::Auto => None,
                EncoderPreference::Named(n) => Some(n.clone()),
            },
        },
    )
    .map_err(map_encoder_error)?;
    let target = match config.source.kind {
        SourceKind::Window => CaptureTarget::Window(config.source.id.0 as isize),
        SourceKind::Monitor => CaptureTarget::Monitor(config.source.id.0 as isize),
    };
    let lost = Arc::clone(shared);
    let capturer = Capturer::start(
        &device,
        target,
        CaptureOptions {
            fps: config.quality.fps,
            cursor: true,
        },
        Arc::clone(&shared.slot),
        Box::new(move || {
            lost.source_lost.store(true, Ordering::SeqCst);
            lost.slot.wake();
        }),
    )
    .map_err(map_capture_error)?;
    Ok(Gpu {
        capturer,
        encoder,
        _device: device,
    })
}

fn run(
    config: SessionConfig,
    shared: Arc<Shared>,
    events: Events,
    controls: Receiver<LiveChange>,
    init_tx: SyncSender<Result<()>>,
) {
    vellcast_encoder::init_thread();
    let reason = match Pipeline::open(&config, &shared, &events, controls) {
        Ok(mut pipeline) => {
            let _ = init_tx.send(Ok(()));
            let reason = pipeline.run_loop(&shared, &events);
            pipeline.close();
            reason
        }
        Err(err) => {
            let _ = init_tx.send(Err(err));
            return;
        }
    };
    events(SessionEvent::Stopped { reason });
}

struct Pipeline {
    config: SessionConfig,
    gpu: Gpu,
    out_w: u32,
    out_h: u32,
    origin_hns: i64,
    /// Origem QPC bruta (para iniciar o áudio depois, durante a transmissão).
    origin_ticks: i64,
    runtime: tokio::runtime::Runtime,
    publisher: Arc<Publisher>,
    audio: Option<vellcast_audio::AudioCapture>,
    /// Microfone pedido pelo usuário (desligado por padrão).
    microphone: Option<MicrophoneConfig>,
    /// Mudanças ao vivo pedidas pela interface.
    controls: Receiver<LiveChange>,
    /// Desde quando a fonte sumiu (janela fechada, monitor desconectado).
    waiting_source: Option<Instant>,
    video_codec: Option<String>,
    inflight: VecDeque<(i64, Instant)>,
    device_resets: u32,
    /// Último quadro capturado, reenviado ao encoder quando a fonte fica parada
    /// (o WGC só entrega quadros quando o conteúdo muda). Mantém o ritmo de FPS,
    /// os keyframes periódicos e a imagem para quem entra depois.
    last_frame: Option<CapturedFrame>,
    last_submit: Instant,
    /// Adaptação de bitrate/FPS (S-001/S-002).
    adapter: Adapter,
    started: Instant,
    last_adapt: Instant,
    /// FPS efetivo enviado ao encoder (≤ FPS da captura).
    target_fps: u32,
    /// Totais acumulados na última decisão, para calcular os deltas da janela.
    prev_network_dropped: u64,
    prev_encoder_dropped: u64,
    /// `Some(false)` quando o encoder recusou mudar o bitrate em tempo real.
    runtime_bitrate: Option<bool>,
    last_adapt_warning: Option<Instant>,
}

impl Pipeline {
    fn open(
        config: &SessionConfig,
        shared: &Arc<Shared>,
        events: &Events,
        controls: Receiver<LiveChange>,
    ) -> Result<Self> {
        let (out_w, out_h) = fit_dimensions(
            config.source.width,
            config.source.height,
            config.quality.max_width,
            config.quality.max_height,
        );
        // Origem comum de vídeo e áudio: instante de início da sessão no relógio
        // QPC, definida antes da captura começar para que nenhum quadro fique
        // com timestamp anterior à origem.
        let origin_ticks = qpc_ticks();
        let origin_hns = ticks_to_hns(origin_ticks, qpc_frequency());
        let gpu = open_gpu(config, shared, out_w, out_h)?;

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("vellcast-net")
            .enable_all()
            .build()
            .map_err(|e| CoreError::InvalidConfig(format!("runtime de rede: {e}")))?;
        let (publisher, mut publisher_events) = {
            let _guard = runtime.enter();
            let mut publisher_config =
                PublisherConfig::new(config.relay_ws_url.clone(), config.publisher_token.clone());
            publisher_config.display_name = config.display_name.clone();
            Publisher::connect(publisher_config)
        };
        let publisher = Arc::new(publisher);

        let net_shared = Arc::clone(shared);
        let net_events = Arc::clone(events);
        runtime.spawn(async move {
            while let Some(event) = publisher_events.recv().await {
                match event {
                    PublisherEvent::Connected { .. } => net_events(SessionEvent::Connected),
                    PublisherEvent::Disconnected { reason } => {
                        net_events(SessionEvent::Reconnecting { reason })
                    }
                    PublisherEvent::KeyframeRequested => {
                        net_shared.keyframe.store(true, Ordering::Relaxed)
                    }
                    PublisherEvent::Fatal { message, .. } => {
                        *net_shared.fatal.lock().unwrap_or_else(|p| p.into_inner()) = Some(message);
                        net_shared.stop.store(true, Ordering::SeqCst);
                        net_shared.slot.wake();
                    }
                }
            }
        });

        let audio = start_audio(config, origin_ticks, &publisher, events);

        events(SessionEvent::Started {
            encoder: gpu.encoder.descriptor().name.clone(),
            width: out_w,
            height: out_h,
            fps: config.quality.fps,
        });
        shared
            .stats
            .set_targets(config.quality.bitrate_kbps, config.quality.fps);
        if !gpu.encoder.descriptor().hardware {
            events(SessionEvent::Warning {
                message: "Nenhum encoder de hardware disponível: usando codificação por software, que consome CPU."
                    .into(),
            });
        }

        Ok(Self {
            config: config.clone(),
            gpu,
            out_w,
            out_h,
            origin_hns,
            origin_ticks,
            runtime,
            publisher,
            audio,
            microphone: None,
            controls,
            waiting_source: None,
            video_codec: None,
            inflight: VecDeque::with_capacity(MAX_INFLIGHT),
            device_resets: 0,
            last_frame: None,
            last_submit: Instant::now(),
            adapter: Adapter::new(
                config.adaptation,
                config.quality.bitrate_kbps,
                config.quality.fps,
                0,
            ),
            started: Instant::now(),
            last_adapt: Instant::now(),
            target_fps: config.quality.fps,
            prev_network_dropped: 0,
            prev_encoder_dropped: 0,
            runtime_bitrate: None,
            last_adapt_warning: None,
        })
    }

    fn run_loop(&mut self, shared: &Arc<Shared>, events: &Events) -> Option<String> {
        let mut out: Vec<EncodedFrame> = Vec::with_capacity(4);
        let mut window = RateWindow::new(Instant::now());
        loop {
            if shared.stop.load(Ordering::SeqCst) {
                return shared
                    .fatal
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
            }
            if let Some(fatal) = self.process_controls(shared, events) {
                return Some(fatal);
            }
            if shared.source_lost.load(Ordering::SeqCst) && self.waiting_source.is_none() {
                self.enter_waiting_source(shared, events);
            }
            if let Some(since) = self.waiting_source {
                // Sem fonte: a conexão e o áudio continuam; espera outra fonte.
                if since.elapsed() >= SOURCE_LOST_GRACE {
                    return Some(
                        "a janela ou o monitor capturado foi fechado e nenhuma outra fonte foi escolhida"
                            .into(),
                    );
                }
                let _ = shared.slot.take(WAITING_SOURCE_POLL);
                window.maybe_publish(Instant::now(), &shared.stats, || self.side_stats());
                continue;
            }

            // Espera curta quando há saída pendente no encoder; senão aguarda o próximo quadro.
            // Com um quadro guardado, acorda a cada meio intervalo para poder repeti-lo.
            let idle = if self.last_frame.is_some() {
                (Duration::from_secs(1) / self.target_fps.max(1) / 2).min(Duration::from_millis(50))
            } else {
                Duration::from_millis(50)
            };
            let timeout = if self.inflight.is_empty() {
                idle
            } else {
                Duration::from_millis(2)
            };
            let frame = shared.slot.take(timeout);
            let step = self.step(frame, shared, &mut out, &mut window);
            if let Err(err) = step {
                match err {
                    EncoderError::DeviceLost if self.device_resets < MAX_DEVICE_RESETS => {
                        self.device_resets += 1;
                        events(SessionEvent::Warning {
                            message:
                                "O dispositivo gráfico foi reiniciado; retomando a transmissão."
                                    .into(),
                        });
                        if let Err(e) = self.reopen_gpu(shared) {
                            return Some(format!("falha ao recuperar o dispositivo gráfico: {e}"));
                        }
                    }
                    other => return Some(format!("falha no encoder: {other}")),
                }
            }
            self.flush_outputs(&mut out, shared, &mut window);
            window.maybe_publish(Instant::now(), &shared.stats, || self.side_stats());
            if self.last_adapt.elapsed() >= ADAPT_INTERVAL {
                self.adapt(shared, events);
            }
        }
    }

    /// Uma decisão da adaptação de qualidade por segundo (S-001/S-002).
    fn adapt(&mut self, shared: &Shared, events: &Events) {
        self.last_adapt = Instant::now();
        let net = self.publisher.stats();
        let live = shared.stats.snapshot();
        let signals = Signals {
            queue_bytes: net.queue_bytes,
            network_dropped: net.video_dropped.saturating_sub(self.prev_network_dropped),
            encoder_dropped: live
                .frames_dropped
                .saturating_sub(self.prev_encoder_dropped),
            rtt_ms: (net.last_rtt_us > 0).then(|| net.last_rtt_us as f32 / 1000.0),
            encode_ms: live.encode_ms_avg,
        };
        self.prev_network_dropped = net.video_dropped;
        self.prev_encoder_dropped = live.frames_dropped;
        let now_ms = self.started.elapsed().as_millis() as u64;
        let Some(change) = self.adapter.update(now_ms, signals) else {
            return;
        };
        tracing::info!(
            bitrate_kbps = change.bitrate_kbps,
            fps = change.fps,
            reason = ?change.reason,
            "adaptação de qualidade"
        );
        if self.runtime_bitrate != Some(false)
            && change.bitrate_kbps != self.gpu.encoder.config().bitrate_kbps
        {
            let ok = self.gpu.encoder.set_bitrate(change.bitrate_kbps);
            if !ok && self.runtime_bitrate.is_none() {
                tracing::warn!(
                    "o encoder não aceita mudar o bitrate em tempo real; só o FPS será ajustado"
                );
            }
            self.runtime_bitrate = Some(ok);
        }
        self.target_fps = change.fps.min(self.config.quality.fps).max(1);
        let applied_bitrate = self.gpu.encoder.config().bitrate_kbps;
        shared.stats.set_targets(applied_bitrate, self.target_fps);

        if change.reason.is_degradation()
            && self
                .last_adapt_warning
                .is_none_or(|t| t.elapsed() >= ADAPT_WARNING_INTERVAL)
        {
            self.last_adapt_warning = Some(Instant::now());
            let message = match change.reason {
                Reason::NetworkCongestion => format!(
                    "Rede lenta até o servidor: qualidade reduzida para {} kbps.",
                    applied_bitrate
                ),
                Reason::EncoderOverload => format!(
                    "Encoder sobrecarregado: qualidade reduzida para {} kbps e {} FPS para preservar o jogo.",
                    applied_bitrate, self.target_fps
                ),
                Reason::RecommendLowerResolution => {
                    "O encoder continua sobrecarregado no FPS mínimo. Para preservar o jogo, transmita de novo com uma resolução menor (perfil Econômico).".to_string()
                }
                Reason::Recovery => String::new(),
            };
            events(SessionEvent::Warning { message });
        }
    }

    fn step(
        &mut self,
        frame: Option<CapturedFrame>,
        shared: &Shared,
        out: &mut Vec<EncodedFrame>,
        window: &mut RateWindow,
    ) -> std::result::Result<(), EncoderError> {
        self.gpu.encoder.poll(out)?;
        let (frame, pts_hns) = match frame {
            Some(frame) => {
                window.captured += 1;
                // FPS reduzido pela adaptação: descarta quadros que chegam antes do
                // próximo intervalo (o mais recente fica guardado para repetição).
                if self.target_fps < self.config.quality.fps {
                    let interval = Duration::from_secs(1) / self.target_fps.max(1);
                    if self.last_submit.elapsed() < interval.mul_f32(0.85) {
                        self.last_frame = Some(frame);
                        return Ok(());
                    }
                }
                let pts = (frame.time_hns - self.origin_hns).max(0);
                (frame, pts)
            }
            None => {
                // Fonte parada: repete o último quadro no ritmo do FPS-alvo.
                let interval = Duration::from_secs(1) / self.target_fps.max(1);
                if self.last_submit.elapsed() < interval + interval / 2 {
                    return Ok(());
                }
                let Some(last) = self.last_frame.take() else {
                    return Ok(());
                };
                let now = ticks_to_hns(qpc_ticks(), qpc_frequency());
                (last, (now - self.origin_hns).max(0))
            }
        };
        if !self.gpu.encoder.ready_for_input() {
            shared.stats.add_dropped(1);
            self.last_frame = Some(frame);
            return Ok(());
        }
        let force = shared.keyframe.swap(false, Ordering::Relaxed);
        // Marca o início antes do envio: encoders síncronos (software) já devolvem
        // o quadro codificado dentro do próprio `submit`.
        let started = Instant::now();
        let outcome = self.gpu.encoder.submit(
            &frame.texture,
            frame.width,
            frame.height,
            pts_hns,
            force,
            out,
        )?;
        match outcome {
            SubmitOutcome::Accepted => {
                if self.inflight.len() == MAX_INFLIGHT {
                    self.inflight.pop_front();
                }
                self.inflight.push_back((pts_hns, started));
                self.last_submit = Instant::now();
            }
            SubmitOutcome::Busy => {
                shared.stats.add_dropped(1);
                if force {
                    shared.keyframe.store(true, Ordering::Relaxed);
                }
            }
        }
        // O quadro fica guardado para repetição; o anterior volta ao pool da captura.
        self.last_frame = Some(frame);
        Ok(())
    }

    fn flush_outputs(
        &mut self,
        out: &mut Vec<EncodedFrame>,
        shared: &Shared,
        window: &mut RateWindow,
    ) {
        for frame in out.drain(..) {
            if let Some(pos) = self
                .inflight
                .iter()
                .position(|(pts, _)| *pts == frame.pts_hns)
            {
                if let Some((_, started)) = self.inflight.remove(pos) {
                    window.add_encode_time(started.elapsed());
                }
                // Entradas mais antigas que esta não terão saída própria (descartadas pelo encoder).
                self.inflight.drain(..pos);
            }
            let codec = self.gpu.encoder.codec_string();
            if codec.is_some() && codec != self.video_codec {
                if let Some(codec) = &codec {
                    self.publisher.set_video_init(&VideoInit {
                        codec: codec.clone(),
                        coded_width: self.out_w,
                        coded_height: self.out_h,
                        framerate: self.config.quality.fps,
                    });
                }
                self.video_codec = codec;
            }
            if self.video_codec.is_none() {
                // Sem SPS ainda não há como configurar o decoder do espectador.
                shared.keyframe.store(true, Ordering::Relaxed);
                continue;
            }
            let pts_us = (frame.pts_hns / 10).max(0) as u64;
            if self
                .publisher
                .send_video(&frame.data, pts_us, frame.keyframe)
                == SendOutcome::Dropped
            {
                shared.keyframe.store(true, Ordering::Relaxed);
            }
            window.encoded += 1;
            window.bytes += frame.data.len() as u64;
        }
    }

    fn side_stats(&self) -> (vellcast_transport::StatsSnapshot, AudioSnapshot) {
        use vellcast_audio::MicState;
        let audio = match &self.audio {
            Some(a) => {
                let s = a.stats();
                AudioSnapshot {
                    active: s.active,
                    game_audio: s.loopback,
                    microphone: match s.mic {
                        MicState::Off => MicStatus::Off,
                        MicState::Starting => MicStatus::Starting,
                        MicState::Active if s.mic_muted => MicStatus::Muted,
                        MicState::Active => MicStatus::Active,
                        MicState::Unavailable => MicStatus::Unavailable,
                    },
                }
            }
            None => AudioSnapshot {
                active: false,
                game_audio: false,
                // Microfone pedido sem captura de áudio: ela não pôde iniciar.
                microphone: if self.microphone.is_some() {
                    MicStatus::Unavailable
                } else {
                    MicStatus::Off
                },
            },
        };
        (self.publisher.stats(), audio)
    }

    /// Aplica as mudanças ao vivo pendentes. `Some` = erro fatal.
    fn process_controls(&mut self, shared: &Arc<Shared>, events: &Events) -> Option<String> {
        while let Ok(change) = self.controls.try_recv() {
            tracing::info!(?change, "mudança durante a transmissão");
            if let Err(fatal) = self.apply_change(change, shared, events) {
                return Some(fatal);
            }
        }
        None
    }

    fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn apply_change(
        &mut self,
        change: LiveChange,
        shared: &Arc<Shared>,
        events: &Events,
    ) -> std::result::Result<(), String> {
        match change {
            LiveChange::Bitrate(kbps) => self.change_bitrate(kbps, shared, events)?,
            LiveChange::Adaptation(mode) => {
                self.config.adaptation = mode;
                let now = self.now_ms();
                self.adapter.set_mode(mode, now);
                self.push_bitrate(shared);
                self.target_fps = self.adapter.fps().min(self.config.quality.fps).max(1);
                shared
                    .stats
                    .set_targets(self.gpu.encoder.config().bitrate_kbps, self.target_fps);
            }
            LiveChange::Video { source, quality } => {
                self.restart_video(source, quality, shared, events)?
            }
            LiveChange::Audio(mode) => {
                self.config.audio = mode;
                self.sync_audio(events);
            }
            LiveChange::Microphone(mic) => {
                self.microphone = mic.clone();
                match &self.audio {
                    Some(audio) => audio.set_microphone(mic.map(to_audio_mic)),
                    None => self.sync_audio(events),
                }
            }
            LiveChange::MicrophoneMuted(muted) => {
                if let Some(mic) = &mut self.microphone {
                    mic.muted = muted;
                }
                if let Some(audio) = &self.audio {
                    audio.set_mic_muted(muted);
                }
            }
        }
        Ok(())
    }

    /// Leva o bitrate da adaptação ao encoder. `false` se o encoder recusar.
    fn push_bitrate(&mut self, shared: &Shared) -> bool {
        let want = self.adapter.bitrate_kbps();
        if want == self.gpu.encoder.config().bitrate_kbps {
            return true;
        }
        if self.runtime_bitrate == Some(false) {
            return false;
        }
        let ok = self.gpu.encoder.set_bitrate(want);
        self.runtime_bitrate = Some(ok);
        shared.stats.set_live_bitrate(Some(ok));
        ok
    }

    fn change_bitrate(
        &mut self,
        kbps: u32,
        shared: &Arc<Shared>,
        events: &Events,
    ) -> std::result::Result<(), String> {
        self.config.quality.bitrate_kbps = kbps;
        let now = self.now_ms();
        self.adapter.set_max_bitrate(kbps, now);
        if !self.push_bitrate(shared) && self.waiting_source.is_none() {
            events(SessionEvent::Warning {
                message: "Este encoder não muda o bitrate em tempo real: reiniciando o encoder com o novo valor.".into(),
            });
            let (source, quality) = (self.config.source.clone(), self.config.quality);
            return self.restart_video(source, quality, shared, events);
        }
        shared
            .stats
            .set_targets(self.gpu.encoder.config().bitrate_kbps, self.target_fps);
        Ok(())
    }

    /// A fonte sumiu: para a captura e espera outra fonte, mantendo a sala.
    fn enter_waiting_source(&mut self, shared: &Shared, events: &Events) {
        tracing::warn!("fonte de captura perdida; aguardando outra fonte");
        self.waiting_source = Some(Instant::now());
        self.gpu.capturer.halt();
        self.last_frame = None;
        while shared.slot.take(Duration::ZERO).is_some() {}
        events(SessionEvent::SourceLost);
    }

    /// Reinicia captura e encoder com outra fonte e/ou qualidade, mantendo a
    /// conexão com o relay (os espectadores recebem um `VideoInit` novo e
    /// retomam no próximo keyframe). Se a nova configuração falhar, volta à
    /// anterior; `Err` só se nem a anterior puder ser reaberta.
    fn restart_video(
        &mut self,
        source: SourceInfo,
        quality: Quality,
        shared: &Arc<Shared>,
        events: &Events,
    ) -> std::result::Result<(), String> {
        let mut next = self.config.clone();
        next.source = source;
        next.quality = quality.clamped();
        let (out_w, out_h) = fit_dimensions(
            next.source.width,
            next.source.height,
            next.quality.max_width,
            next.quality.max_height,
        );
        let previous_pid = self.config.source.process_id;
        self.gpu.capturer.halt();
        self.last_frame = None;
        while shared.slot.take(Duration::ZERO).is_some() {}
        shared.source_lost.store(false, Ordering::SeqCst);
        match open_gpu(&next, shared, out_w, out_h) {
            Ok(fresh) => {
                drop(std::mem::replace(&mut self.gpu, fresh));
                self.config = next;
                self.out_w = out_w;
                self.out_h = out_h;
                self.waiting_source = None;
            }
            Err(e) => {
                tracing::warn!(error = %e, "não foi possível reiniciar o vídeo com a nova configuração");
                events(SessionEvent::Warning {
                    message: format!(
                        "Não foi possível aplicar a mudança ({e}). A configuração anterior foi mantida."
                    ),
                });
                if self.waiting_source.is_some() {
                    // A fonte anterior não existe mais: continua esperando outra.
                    return Ok(());
                }
                let fresh = open_gpu(&self.config, shared, self.out_w, self.out_h)
                    .map_err(|e| format!("falha ao retomar a captura: {e}"))?;
                drop(std::mem::replace(&mut self.gpu, fresh));
            }
        }
        // Encoder novo: força um VideoInit com as dimensões novas e um keyframe.
        self.video_codec = None;
        self.inflight.clear();
        shared.keyframe.store(true, Ordering::Relaxed);
        let now = self.now_ms();
        self.adapter = Adapter::new(
            self.config.adaptation,
            self.config.quality.bitrate_kbps,
            self.config.quality.fps,
            now,
        );
        self.target_fps = self.config.quality.fps;
        self.runtime_bitrate = None;
        shared.stats.set_live_bitrate(None);
        self.last_submit = Instant::now();
        shared
            .stats
            .set_targets(self.config.quality.bitrate_kbps, self.config.quality.fps);
        events(SessionEvent::VideoRestarted {
            source: self.config.source.title.clone(),
            encoder: self.gpu.encoder.descriptor().name.clone(),
            width: self.out_w,
            height: self.out_h,
            fps: self.config.quality.fps,
        });
        if self.config.source.process_id != previous_pid {
            // "Só o aplicativo" acompanha a janela compartilhada.
            self.sync_audio(events);
        }
        Ok(())
    }

    /// Origem do áudio do jogo para o modo e a fonte atuais.
    fn audio_target(&self) -> Option<vellcast_audio::AudioTarget> {
        use vellcast_audio::AudioTarget;
        match (self.config.audio, self.config.source.process_id) {
            (AudioMode::Off, _) => None,
            (AudioMode::Application, Some(pid)) => Some(AudioTarget::Process(pid)),
            (AudioMode::Application, None) => None,
            (AudioMode::System, _) => Some(AudioTarget::System),
        }
    }

    /// Aplica modo de áudio e microfone atuais, iniciando a captura se preciso.
    fn sync_audio(&mut self, events: &Events) {
        let target = self.audio_target();
        if self.config.audio == AudioMode::Application && target.is_none() {
            events(SessionEvent::Warning {
                message: "Monitor compartilhado: não há um aplicativo para capturar o áudio. Escolha \"Sistema inteiro\" para compartilhar o som.".into(),
            });
        }
        let mic = self.microphone.clone().map(to_audio_mic);
        match &self.audio {
            Some(audio) => audio.set_loopback(target),
            None if target.is_some() || mic.is_some() => {
                self.audio =
                    start_audio_live(target, mic, self.origin_ticks, &self.publisher, events);
            }
            None => {}
        }
    }

    fn reopen_gpu(&mut self, shared: &Arc<Shared>) -> Result<()> {
        // Para a captura antiga e descarta quadros pendentes do dispositivo perdido
        // antes de criar os recursos novos.
        self.gpu.capturer.halt();
        // O quadro guardado pertence ao dispositivo perdido.
        self.last_frame = None;
        while shared.slot.take(Duration::ZERO).is_some() {}
        let fresh = open_gpu(&self.config, shared, self.out_w, self.out_h)?;
        drop(std::mem::replace(&mut self.gpu, fresh));
        // O encoder novo nasce com o bitrate configurado; reaplica o alvo da adaptação.
        if self.runtime_bitrate != Some(false)
            && self.adapter.bitrate_kbps() != self.gpu.encoder.config().bitrate_kbps
        {
            self.gpu.encoder.set_bitrate(self.adapter.bitrate_kbps());
        }
        self.inflight.clear();
        shared.keyframe.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn close(self) {
        let Self {
            gpu,
            runtime,
            publisher,
            audio,
            ..
        } = self;
        gpu.capturer.stop();
        if let Some(audio) = audio {
            audio.stop();
        }
        drop(gpu.encoder);
        // Encerra a conexão com StreamEnd para a sala voltar a ociosa na hora.
        match Arc::try_unwrap(publisher) {
            Ok(publisher) => runtime.block_on(publisher.finish(Duration::from_secs(2))),
            Err(publisher) => drop(publisher),
        }
        runtime.shutdown_timeout(Duration::from_secs(2));
    }
}

fn start_audio(
    config: &SessionConfig,
    origin_ticks: i64,
    publisher: &Arc<Publisher>,
    events: &Events,
) -> Option<vellcast_audio::AudioCapture> {
    use vellcast_audio::{AudioCapture, AudioError, AudioTarget};

    let target = match (config.audio, config.source.process_id) {
        (AudioMode::Off, _) => return None,
        (AudioMode::Application, Some(pid)) => AudioTarget::Process(pid),
        (AudioMode::Application, None) => {
            events(SessionEvent::Warning {
                message: "Monitor selecionado: transmitindo o áudio de todo o sistema.".into(),
            });
            AudioTarget::System
        }
        (AudioMode::System, _) => AudioTarget::System,
    };

    publisher.set_audio_init(&AudioInit {
        codec: "opus".into(),
        sample_rate: vellcast_audio::SAMPLE_RATE,
        number_of_channels: vellcast_audio::CHANNELS,
    });

    let try_start = |target: AudioTarget| {
        let sink = Arc::clone(publisher);
        let warn_events = Arc::clone(events);
        AudioCapture::start(
            target,
            origin_ticks,
            Box::new(move |packet| {
                let _ = sink.send_audio(&packet.data, packet.pts_us);
            }),
            Box::new(move |message| warn_events(SessionEvent::Warning { message })),
        )
    };

    match try_start(target) {
        Ok(capture) => Some(capture),
        Err(AudioError::ProcessLoopbackUnavailable(detail)) => {
            events(SessionEvent::Warning {
                message: format!(
                    "Áudio só do jogo indisponível ({detail}). Transmitindo o áudio de todo o sistema."
                ),
            });
            match try_start(AudioTarget::System) {
                Ok(capture) => Some(capture),
                Err(err) => {
                    events(SessionEvent::Warning {
                        message: format!("Áudio desativado: {err}"),
                    });
                    None
                }
            }
        }
        Err(err) => {
            events(SessionEvent::Warning {
                message: format!("Áudio desativado: {err}"),
            });
            None
        }
    }
}

fn to_audio_mic(mic: MicrophoneConfig) -> vellcast_audio::MicConfig {
    vellcast_audio::MicConfig {
        device_id: mic.device_id,
        muted: mic.muted,
    }
}

/// Inicia o áudio durante a transmissão (áudio do jogo ligado depois, ou
/// microfone). Não bloqueia o pipeline: começa só com silêncio; o loopback e
/// o microfone abrem na thread de áudio.
fn start_audio_live(
    target: Option<vellcast_audio::AudioTarget>,
    microphone: Option<vellcast_audio::MicConfig>,
    origin_ticks: i64,
    publisher: &Arc<Publisher>,
    events: &Events,
) -> Option<vellcast_audio::AudioCapture> {
    use vellcast_audio::{AudioCapture, AudioSetup};

    publisher.set_audio_init(&AudioInit {
        codec: "opus".into(),
        sample_rate: vellcast_audio::SAMPLE_RATE,
        number_of_channels: vellcast_audio::CHANNELS,
    });
    let sink = Arc::clone(publisher);
    let warn_events = Arc::clone(events);
    let started = AudioCapture::start_with(
        AudioSetup {
            loopback: None,
            microphone,
        },
        origin_ticks,
        Box::new(move |packet| {
            let _ = sink.send_audio(&packet.data, packet.pts_us);
        }),
        Box::new(move |message| warn_events(SessionEvent::Warning { message })),
    );
    match started {
        Ok(capture) => {
            if target.is_some() {
                capture.set_loopback(target);
            }
            Some(capture)
        }
        Err(err) => {
            events(SessionEvent::Warning {
                message: format!("Áudio desativado: {err}"),
            });
            None
        }
    }
}
