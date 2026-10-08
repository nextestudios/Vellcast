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
use std::sync::mpsc::{SyncSender, sync_channel};
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

use self::stats::{LiveStats, RateWindow};
use super::*;

/// Intervalo entre keyframes automáticos.
const KEYFRAME_INTERVAL_S: u32 = 2;
/// Tempo máximo para o pipeline inicializar antes de `start` retornar erro.
const INIT_TIMEOUT: Duration = Duration::from_secs(15);
/// Reinícios após perda do dispositivo gráfico antes de desistir.
const MAX_DEVICE_RESETS: u32 = 3;
/// Quadros em codificação rastreados para medir o tempo de codificação.
const MAX_INFLIGHT: usize = 16;

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
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("vellcast-pipeline".into())
            .spawn(move || run(config, thread_shared, Arc::new(events), init_tx))
            .map_err(|e| CoreError::Capture(format!("falha ao criar a thread do pipeline: {e}")))?;

        let session = Self {
            shared,
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
    init_tx: SyncSender<Result<()>>,
) {
    vellcast_encoder::init_thread();
    let reason = match Pipeline::open(&config, &shared, &events) {
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
    runtime: tokio::runtime::Runtime,
    publisher: Arc<Publisher>,
    audio: Option<vellcast_audio::AudioCapture>,
    video_codec: Option<String>,
    inflight: VecDeque<(i64, Instant)>,
    device_resets: u32,
}

impl Pipeline {
    fn open(config: &SessionConfig, shared: &Arc<Shared>, events: &Events) -> Result<Self> {
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
            Publisher::connect(PublisherConfig::new(
                config.relay_ws_url.clone(),
                config.publisher_token.clone(),
            ))
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
            runtime,
            publisher,
            audio,
            video_codec: None,
            inflight: VecDeque::with_capacity(MAX_INFLIGHT),
            device_resets: 0,
        })
    }

    fn run_loop(&mut self, shared: &Arc<Shared>, events: &Events) -> Option<String> {
        let mut out: Vec<EncodedFrame> = Vec::with_capacity(4);
        let mut window = RateWindow::new(Instant::now());
        loop {
            if shared.source_lost.load(Ordering::SeqCst) {
                events(SessionEvent::SourceLost);
                return Some("a janela ou o monitor capturado foi fechado".into());
            }
            if shared.stop.load(Ordering::SeqCst) {
                return shared
                    .fatal
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .take();
            }

            // Espera curta quando há saída pendente no encoder; senão aguarda o próximo quadro.
            let timeout = if self.inflight.is_empty() {
                Duration::from_millis(50)
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
        let Some(frame) = frame else { return Ok(()) };
        window.captured += 1;
        if !self.gpu.encoder.ready_for_input() {
            shared.stats.add_dropped(1);
            return Ok(());
        }
        let pts_hns = (frame.time_hns - self.origin_hns).max(0);
        let force = shared.keyframe.swap(false, Ordering::Relaxed);
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
                self.inflight.push_back((pts_hns, Instant::now()));
            }
            SubmitOutcome::Busy => {
                shared.stats.add_dropped(1);
                if force {
                    shared.keyframe.store(true, Ordering::Relaxed);
                }
            }
        }
        // `frame` é descartado aqui e o buffer volta ao pool da captura.
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

    fn side_stats(&self) -> (vellcast_transport::StatsSnapshot, bool) {
        let audio_active = self.audio.as_ref().is_some_and(|a| a.stats().active);
        (self.publisher.stats(), audio_active)
    }

    fn reopen_gpu(&mut self, shared: &Arc<Shared>) -> Result<()> {
        // Para a captura antiga e descarta quadros pendentes do dispositivo perdido
        // antes de criar os recursos novos.
        self.gpu.capturer.halt();
        while shared.slot.take(Duration::ZERO).is_some() {}
        let fresh = open_gpu(&self.config, shared, self.out_w, self.out_h)?;
        drop(std::mem::replace(&mut self.gpu, fresh));
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
