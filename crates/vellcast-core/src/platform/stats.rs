//! Métricas da sessão: o pipeline publica valores a cada segundo em atômicos
//! e a interface lê sem bloquear a thread de codificação.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::{MicStatus, SessionStats};

/// Estado do áudio publicado junto com as métricas.
#[derive(Debug, Clone, Copy)]
pub(super) struct AudioSnapshot {
    pub active: bool,
    pub game_audio: bool,
    pub microphone: MicStatus,
}

fn mic_to_u8(m: MicStatus) -> u8 {
    match m {
        MicStatus::Off => 0,
        MicStatus::Starting => 1,
        MicStatus::Active => 2,
        MicStatus::Muted => 3,
        MicStatus::Unavailable => 4,
    }
}

fn mic_from_u8(v: u8) -> MicStatus {
    match v {
        1 => MicStatus::Starting,
        2 => MicStatus::Active,
        3 => MicStatus::Muted,
        4 => MicStatus::Unavailable,
        _ => MicStatus::Off,
    }
}

fn f32_store(a: &AtomicU32, v: f32) {
    a.store(v.to_bits(), Ordering::Relaxed);
}

fn f32_load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

pub(super) struct LiveStats {
    started: Instant,
    captured_fps: AtomicU32,
    encoded_fps: AtomicU32,
    bitrate_kbps: AtomicU32,
    encode_ms: AtomicU32,
    frames_dropped: AtomicU64,
    network_dropped: AtomicU64,
    reconnects: AtomicU64,
    /// RTT em µs; 0 = desconhecido.
    rtt_us: AtomicU64,
    audio_active: AtomicBool,
    game_audio: AtomicBool,
    microphone: AtomicU8,
    /// 0 = desconhecido, 1 = aceita, 2 = recusa mudar o bitrate em tempo real.
    live_bitrate: AtomicU8,
    target_bitrate_kbps: AtomicU32,
    target_fps: AtomicU32,
}

impl LiveStats {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            captured_fps: AtomicU32::new(0),
            encoded_fps: AtomicU32::new(0),
            bitrate_kbps: AtomicU32::new(0),
            encode_ms: AtomicU32::new(0),
            frames_dropped: AtomicU64::new(0),
            network_dropped: AtomicU64::new(0),
            reconnects: AtomicU64::new(0),
            rtt_us: AtomicU64::new(0),
            audio_active: AtomicBool::new(false),
            game_audio: AtomicBool::new(false),
            microphone: AtomicU8::new(0),
            live_bitrate: AtomicU8::new(0),
            target_bitrate_kbps: AtomicU32::new(0),
            target_fps: AtomicU32::new(0),
        }
    }

    /// Alvo atual definido pela adaptação de qualidade.
    pub fn set_targets(&self, bitrate_kbps: u32, fps: u32) {
        self.target_bitrate_kbps
            .store(bitrate_kbps, Ordering::Relaxed);
        self.target_fps.store(fps, Ordering::Relaxed);
    }

    pub fn set_live_bitrate(&self, live: Option<bool>) {
        let v = match live {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        self.live_bitrate.store(v, Ordering::Relaxed);
    }

    pub fn add_dropped(&self, n: u64) {
        self.frames_dropped.fetch_add(n, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> SessionStats {
        let rtt = self.rtt_us.load(Ordering::Relaxed);
        SessionStats {
            uptime: self.started.elapsed(),
            captured_fps: f32_load(&self.captured_fps),
            encoded_fps: f32_load(&self.encoded_fps),
            frames_dropped: self.frames_dropped.load(Ordering::Relaxed),
            network_dropped: self.network_dropped.load(Ordering::Relaxed),
            bitrate_kbps: f32_load(&self.bitrate_kbps),
            encode_ms_avg: f32_load(&self.encode_ms),
            audio_active: self.audio_active.load(Ordering::Relaxed),
            reconnects: self.reconnects.load(Ordering::Relaxed),
            rtt_ms: (rtt > 0).then(|| rtt as f32 / 1000.0),
            target_bitrate_kbps: self.target_bitrate_kbps.load(Ordering::Relaxed),
            target_fps: self.target_fps.load(Ordering::Relaxed),
            game_audio: self.game_audio.load(Ordering::Relaxed),
            microphone: mic_from_u8(self.microphone.load(Ordering::Relaxed)),
            live_bitrate: match self.live_bitrate.load(Ordering::Relaxed) {
                1 => Some(true),
                2 => Some(false),
                _ => None,
            },
        }
    }
}

/// Contadores da janela de 1 s atual.
pub(super) struct RateWindow {
    start: Instant,
    pub captured: u64,
    pub encoded: u64,
    pub bytes: u64,
    encode_total: Duration,
    encode_samples: u32,
}

impl RateWindow {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            captured: 0,
            encoded: 0,
            bytes: 0,
            encode_total: Duration::ZERO,
            encode_samples: 0,
        }
    }

    pub fn add_encode_time(&mut self, d: Duration) {
        self.encode_total += d;
        self.encode_samples += 1;
    }

    /// Publica a janela se já passou 1 s. `side` fornece dados de transporte e áudio.
    pub fn maybe_publish(
        &mut self,
        now: Instant,
        stats: &LiveStats,
        side: impl FnOnce() -> (vellcast_transport::StatsSnapshot, AudioSnapshot),
    ) {
        let elapsed = now.duration_since(self.start);
        if elapsed < Duration::from_secs(1) {
            return;
        }
        let secs = elapsed.as_secs_f32();
        f32_store(&stats.captured_fps, self.captured as f32 / secs);
        f32_store(&stats.encoded_fps, self.encoded as f32 / secs);
        f32_store(&stats.bitrate_kbps, (self.bytes * 8) as f32 / 1000.0 / secs);
        if self.encode_samples > 0 {
            let avg = self.encode_total.as_secs_f32() * 1000.0 / self.encode_samples as f32;
            f32_store(&stats.encode_ms, avg);
        }
        let (net, audio) = side();
        stats
            .network_dropped
            .store(net.video_dropped, Ordering::Relaxed);
        stats.reconnects.store(net.reconnects, Ordering::Relaxed);
        stats.rtt_us.store(net.last_rtt_us, Ordering::Relaxed);
        stats.audio_active.store(audio.active, Ordering::Relaxed);
        stats.game_audio.store(audio.game_audio, Ordering::Relaxed);
        stats
            .microphone
            .store(mic_to_u8(audio.microphone), Ordering::Relaxed);
        *self = Self::new(now);
    }
}
