//! Métricas da sessão: o pipeline publica valores a cada segundo em atômicos
//! e a interface lê sem bloquear a thread de codificação.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::SessionStats;

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
        }
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
        side: impl FnOnce() -> (vellcast_transport::StatsSnapshot, bool),
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
        let (net, audio_active) = side();
        stats
            .network_dropped
            .store(net.video_dropped, Ordering::Relaxed);
        stats.reconnects.store(net.reconnects, Ordering::Relaxed);
        stats.rtt_us.store(net.last_rtt_us, Ordering::Relaxed);
        stats.audio_active.store(audio_active, Ordering::Relaxed);
        *self = Self::new(now);
    }
}
