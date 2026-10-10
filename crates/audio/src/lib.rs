//! Captura de áudio do Vellcast (ETAPA 4, M-009).
//!
//! - Windows: WASAPI. [`AudioTarget::Process`] usa o *Application Loopback*
//!   (`VAD\Process_Loopback`, Windows 10 2004+) incluindo a árvore de processos
//!   do jogo. [`AudioTarget::System`] captura o áudio do sistema excluindo o
//!   Discord (para não transmitir a chamada de voz); sem suporte a loopback por
//!   processo, cai para o loopback clássico do dispositivo padrão e avisa.
//! - Microfone (opcional, desligado por padrão): captura de um dispositivo de
//!   entrada WASAPI numa thread própria, mixada ao bloco do loopback antes do
//!   Opus ([`mix`]). Origem do loopback, microfone e mudo mudam durante a
//!   transmissão sem bloquear quem chama ([`AudioCapture::set_loopback`],
//!   [`AudioCapture::set_microphone`], [`AudioCapture::set_mic_muted`]).
//! - O áudio é convertido para 48 kHz estéreo `f32`, dividido em blocos de
//!   20 ms com timestamps no mesmo relógio (QPC) do vídeo e codificado em Opus.
//! - Fora do Windows, [`AudioCapture::start`] retorna [`AudioError::Unsupported`].
//!
//! Detalhes em `docs/architecture/AUDIO.md`.

pub mod convert;
pub mod frames;
pub mod mix;
pub mod process;

#[cfg(windows)]
mod opus_encoder;
#[cfg(windows)]
mod wasapi;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::mix::MicFifo;

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

/// Microfone a mixar na transmissão (opt-in).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MicConfig {
    /// ID do dispositivo de entrada (de [`list_input_devices`]); `None` = padrão do Windows.
    pub device_id: Option<String>,
    /// Começa silenciado (o dispositivo fica aberto, mas nada é transmitido).
    pub muted: bool,
}

/// O que capturar ao iniciar; tudo pode mudar depois, durante a transmissão.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AudioSetup {
    /// Áudio do aplicativo ou do sistema; `None` = só silêncio (e o microfone, se houver).
    pub loopback: Option<AudioTarget>,
    pub microphone: Option<MicConfig>,
}

/// Dispositivo de entrada (microfone) disponível.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    /// Dispositivo de entrada padrão do Windows.
    pub is_default: bool,
}

/// Estado do microfone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicState {
    #[default]
    Off,
    /// Abrindo o dispositivo (ou reabrindo após uma falha).
    Starting,
    Active,
    /// Dispositivo ausente, desconectado ou recusado; a transmissão segue sem microfone.
    Unavailable,
}

impl MicState {
    fn to_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Starting => 1,
            Self::Active => 2,
            Self::Unavailable => 3,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Starting,
            2 => Self::Active,
            3 => Self::Unavailable,
            _ => Self::Off,
        }
    }
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
    /// Há áudio de aplicativo/sistema sendo capturado (não só silêncio).
    pub loopback: bool,
    pub mic: MicState,
    pub mic_muted: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("captura de áudio disponível apenas no Windows")]
    Unsupported,
    #[error(
        "captura de áudio por aplicativo indisponível (requer Windows 10 2004 ou superior): {0}"
    )]
    ProcessLoopbackUnavailable(String),
    #[error("nenhum dispositivo de áudio disponível: {0}")]
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

/// Lista os microfones (dispositivos de entrada ativos). Vazia quando não há nenhum.
pub fn list_input_devices() -> Result<Vec<InputDevice>, AudioError> {
    #[cfg(windows)]
    {
        wasapi::list_input_devices()
    }
    #[cfg(not(windows))]
    {
        Err(AudioError::Unsupported)
    }
}

/// Pedidos de mudança ainda não aplicados pela thread de captura.
#[derive(Debug, Default)]
pub(crate) struct Requests {
    /// `Some(x)`: trocar o loopback para `x`.
    pub loopback: Option<Option<AudioTarget>>,
    /// `Some(x)`: trocar o microfone para `x`.
    pub microphone: Option<Option<MicConfig>>,
}

/// Estado compartilhado entre a thread de captura e o dono da captura.
#[derive(Debug)]
pub(crate) struct Shared {
    pub stop: AtomicBool,
    pub packets: AtomicU64,
    pub active: AtomicBool,
    pub loopback: AtomicBool,
    /// Ganho linear em bits de `f32`.
    pub gain_bits: AtomicU32,
    /// Há pedidos em `requests` (evita travar o mutex a cada volta do laço).
    pub dirty: AtomicBool,
    pub requests: Mutex<Requests>,
    pub mic_muted: AtomicBool,
    pub mic_state: AtomicU8,
    /// Geração do microfone atual: threads de microfone antigas param de escrever.
    pub mic_generation: AtomicU64,
    pub mic_fifo: Mutex<MicFifo>,
}

impl Shared {
    fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            packets: AtomicU64::new(0),
            active: AtomicBool::new(false),
            loopback: AtomicBool::new(false),
            gain_bits: AtomicU32::new(1.0f32.to_bits()),
            dirty: AtomicBool::new(false),
            requests: Mutex::new(Requests::default()),
            mic_muted: AtomicBool::new(false),
            mic_state: AtomicU8::new(MicState::Off.to_u8()),
            mic_generation: AtomicU64::new(0),
            mic_fifo: Mutex::new(MicFifo::default()),
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }

    pub fn mic_state(&self) -> MicState {
        MicState::from_u8(self.mic_state.load(Ordering::Relaxed))
    }

    /// Atualiza o estado do microfone se `generation` ainda for a atual.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn set_mic_state(&self, generation: u64, state: MicState) {
        if self.mic_generation.load(Ordering::SeqCst) == generation {
            self.mic_state.store(state.to_u8(), Ordering::Relaxed);
        }
    }

    pub fn requests(&self) -> MutexGuard<'_, Requests> {
        self.requests.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn mic_fifo(&self) -> MutexGuard<'_, MicFifo> {
        self.mic_fifo.lock().unwrap_or_else(|p| p.into_inner())
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
        Self::start_with(
            AudioSetup {
                loopback: Some(target),
                microphone: None,
            },
            origin_qpc,
            on_packet,
            on_warning,
        )
    }

    /// Como [`AudioCapture::start`], com loopback e microfone opcionais.
    ///
    /// Com `loopback`, só retorna depois que o dispositivo foi aberto (ou com o
    /// erro). Sem `loopback`, retorna na hora: a linha do tempo começa com
    /// silêncio e o microfone abre em segundo plano (o estado aparece em
    /// [`AudioStats::mic`]).
    pub fn start_with(
        setup: AudioSetup,
        origin_qpc: i64,
        on_packet: PacketSink,
        on_warning: WarningSink,
    ) -> Result<AudioCapture, AudioError> {
        let shared = Arc::new(Shared::new());
        if let Some(mic) = &setup.microphone {
            shared.mic_muted.store(mic.muted, Ordering::Relaxed);
        }
        #[cfg(windows)]
        {
            let thread = wasapi::spawn(
                setup,
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
            let _ = (setup, origin_qpc, on_packet, on_warning, shared);
            Err(AudioError::Unsupported)
        }
    }

    pub fn stats(&self) -> AudioStats {
        AudioStats {
            packets: self.shared.packets.load(Ordering::Relaxed),
            active: self.shared.active.load(Ordering::Relaxed),
            loopback: self.shared.loopback.load(Ordering::Relaxed),
            mic: self.shared.mic_state(),
            mic_muted: self.shared.mic_muted.load(Ordering::Relaxed),
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

    /// Troca a origem do áudio de aplicativo/sistema (`None` desliga). Não
    /// bloqueia: a thread de captura aplica em até ~20 ms. Se o loopback por
    /// processo falhar, cai para o áudio do sistema (excluindo o Discord) e avisa.
    pub fn set_loopback(&self, target: Option<AudioTarget>) {
        self.shared.requests().loopback = Some(target);
        self.shared.dirty.store(true, Ordering::SeqCst);
    }

    /// Liga, troca ou desliga (`None`) o microfone. Não bloqueia: o dispositivo
    /// é aberto numa thread própria.
    pub fn set_microphone(&self, mic: Option<MicConfig>) {
        if let Some(m) = &mic {
            self.shared.mic_muted.store(m.muted, Ordering::Relaxed);
        }
        self.shared.requests().microphone = Some(mic);
        self.shared.dirty.store(true, Ordering::SeqCst);
    }

    /// Silencia ou reativa o microfone sem fechar o dispositivo.
    pub fn set_mic_muted(&self, muted: bool) {
        self.shared.mic_muted.store(muted, Ordering::Relaxed);
        if muted {
            self.shared.mic_fifo().clear();
        }
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
        assert!(matches!(list_input_devices(), Err(AudioError::Unsupported)));
    }

    #[test]
    fn frame_constants_are_consistent() {
        assert_eq!(
            FRAME_SAMPLES as u64 * 1_000_000 / u64::from(SAMPLE_RATE),
            FRAME_DURATION_US
        );
    }

    #[test]
    fn mic_state_round_trips_and_ignores_stale_generations() {
        for s in [
            MicState::Off,
            MicState::Starting,
            MicState::Active,
            MicState::Unavailable,
        ] {
            assert_eq!(MicState::from_u8(s.to_u8()), s);
        }
        let shared = Shared::new();
        shared.mic_generation.store(2, Ordering::SeqCst);
        shared.set_mic_state(1, MicState::Active);
        assert_eq!(shared.mic_state(), MicState::Off, "geração antiga ignorada");
        shared.set_mic_state(2, MicState::Active);
        assert_eq!(shared.mic_state(), MicState::Active);
    }
}
