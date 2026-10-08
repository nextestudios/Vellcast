//! Captura de áudio do Vellcast (ETAPA 4, M-009).
//!
//! - Windows: WASAPI. [`AudioTarget::Process`] usa o *Application Loopback*
//!   (`VAD\Process_Loopback`, Windows 10 2004+) incluindo a árvore de processos
//!   do jogo. [`AudioTarget::System`] captura o áudio do sistema excluindo o
//!   Discord (para não transmitir a chamada de voz); sem suporte a loopback por
//!   processo, cai para o loopback clássico do dispositivo padrão e avisa.
//! - O áudio é convertido para 48 kHz estéreo `f32`, dividido em blocos de
//!   20 ms com timestamps no mesmo relógio (QPC) do vídeo e codificado em Opus.
//! - Fora do Windows, [`AudioCapture::start`] retorna [`AudioError::Unsupported`].
//!
//! Detalhes em `docs/architecture/AUDIO.md`.

pub mod convert;
pub mod frames;
pub mod process;

#[cfg(windows)]
mod opus_encoder;
#[cfg(windows)]
mod wasapi;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

/// Taxa de amostragem de saída (e do Opus).
pub const SAMPLE_RATE: u32 = 48_000;
/// Canais de saída.
pub const CHANNELS: u8 = 2;
/// Amostras por canal em um pacote Opus de 20 ms.
pub const FRAME_SAMPLES: usize = 960;
/// Duração de um pacote em microssegundos.
pub const FRAME_DURATION_US: u64 = 20_000;
/// Bitrate Opus usado na transmissão.
pub const OPUS_BITRATE_BPS: i32 = 128_000;

/// O que capturar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTarget {
    /// Áudio do processo (e de seus filhos), via Application Loopback.
    Process(u32),
    /// Todo o áudio do sistema, excluindo o Discord quando possível.
    System,
}

/// Um pacote Opus de 20 ms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPacket {
    pub data: Vec<u8>,
    /// Microssegundos desde `origin_qpc` (mesmo relógio do vídeo).
    pub pts_us: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AudioStats {
    /// Pacotes Opus entregues.
    pub packets: u64,
    /// A captura está recebendo dados do dispositivo.
    pub active: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("captura de áudio disponível apenas no Windows")]
    Unsupported,
    #[error(
        "captura de áudio por aplicativo indisponível (requer Windows 10 2004 ou superior): {0}"
    )]
    ProcessLoopbackUnavailable(String),
    #[error("nenhum dispositivo de saída de áudio disponível: {0}")]
    NoDevice(String),
    #[error("falha no WASAPI: {0}")]
    Wasapi(String),
    #[error("falha no encoder Opus: {0}")]
    Encoder(String),
    #[error("a captura não respondeu a tempo")]
    Timeout,
}

/// Callback de pacotes; chamado da thread de captura.
pub type PacketSink = Box<dyn FnMut(AudioPacket) + Send>;
/// Callback de avisos não fatais (ex.: Discord pode ser capturado).
pub type WarningSink = Box<dyn Fn(String) + Send + Sync>;

/// Valor atual de `QueryPerformanceCounter` (origem para `AudioCapture::start`).
/// `None` fora do Windows.
pub fn qpc_now() -> Option<i64> {
    #[cfg(windows)]
    {
        Some(wasapi::qpc_ticks())
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Estado compartilhado entre a thread de captura e o dono da captura.
#[derive(Debug)]
pub(crate) struct Shared {
    pub stop: AtomicBool,
    pub packets: AtomicU64,
    pub active: AtomicBool,
    /// Ganho linear em bits de `f32`.
    pub gain_bits: AtomicU32,
}

impl Shared {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            packets: AtomicU64::new(0),
            active: AtomicBool::new(false),
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }
}

/// Uma captura de áudio ativa. Encerrada por [`AudioCapture::stop`] ou ao ser descartada.
pub struct AudioCapture {
    shared: Arc<Shared>,
    #[cfg(windows)]
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioCapture {
    /// Inicia a captura. Retorna depois que o dispositivo foi aberto (ou com o erro).
    ///
    /// `origin_qpc` é o valor bruto de `QueryPerformanceCounter` que define o
    /// instante zero da transmissão; os `pts_us` dos pacotes são relativos a ele.
    pub fn start(
        target: AudioTarget,
        origin_qpc: i64,
        on_packet: PacketSink,
        on_warning: WarningSink,
    ) -> Result<AudioCapture, AudioError> {
        let shared = Arc::new(Shared::new());
        #[cfg(windows)]
        {
            let thread = wasapi::spawn(
                target,
                origin_qpc,
                on_packet,
                on_warning,
                Arc::clone(&shared),
            )?;
            Ok(AudioCapture {
                shared,
                thread: Some(thread),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = (target, origin_qpc, on_packet, on_warning, shared);
            Err(AudioError::Unsupported)
        }
    }

    pub fn stats(&self) -> AudioStats {
        AudioStats {
            packets: self.shared.packets.load(Ordering::Relaxed),
            active: self.shared.active.load(Ordering::Relaxed),
        }
    }

    /// Ganho linear aplicado antes da codificação (0.0 a 2.0).
    pub fn set_volume(&self, gain: f32) {
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 2.0)
        } else {
            1.0
        };
        self.shared
            .gain_bits
            .store(gain.to_bits(), Ordering::Relaxed);
    }

    /// Para a captura e espera a thread terminar.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        #[cfg(windows)]
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn unsupported_outside_windows() {
        let r = AudioCapture::start(AudioTarget::System, 0, Box::new(|_| {}), Box::new(|_| {}));
        assert!(matches!(r, Err(AudioError::Unsupported)));
    }

    #[test]
    fn frame_constants_are_consistent() {
        assert_eq!(
            FRAME_SAMPLES as u64 * 1_000_000 / u64::from(SAMPLE_RATE),
            FRAME_DURATION_US
        );
    }
}
