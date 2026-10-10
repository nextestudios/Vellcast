//! API pública do motor de transmissão do Vellcast.
//!
//! A interface (apps/desktop) só conhece este crate. Captura (`capture`),
//! codificação (`encoder`), áudio (`audio`) e transporte (`transport`) ficam
//! atrás de [`StreamSession`]. Fora do Windows, todas as operações retornam
//! [`CoreError::Unsupported`], o que permite desenvolver a interface no macOS.

use std::time::Duration;

pub mod adapt;
pub mod live;

pub use adapt::Adaptation;
pub use live::ContentPreference;

/// Identificador opaco de uma fonte (HWND ou HMONITOR no Windows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Window,
    Monitor,
}

/// Uma janela ou monitor que o usuário pode escolher explicitamente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    pub id: SourceId,
    pub kind: SourceKind,
    /// Título da janela ou nome do monitor.
    pub title: String,
    /// Nome do executável (somente janelas).
    pub process_name: Option<String>,
    /// PID do processo dono da janela (usado na captura de áudio por processo).
    pub process_id: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// Heurística não invasiva de jogo (S-003): tela cheia/sem bordas, executável conhecido etc.
    pub likely_game: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderInfo {
    /// Nome amigável, ex.: "NVIDIA NVENC H.264".
    pub name: String,
    pub vendor: EncoderVendor,
    pub hardware: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderVendor {
    Nvidia,
    Amd,
    Intel,
    Microsoft,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EncoderPreference {
    /// Primeiro encoder de hardware disponível; software só como último recurso.
    #[default]
    Auto,
    /// Encoder pelo nome retornado em [`list_encoders`].
    Named(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioMode {
    Off,
    /// Somente o áudio do processo da janela selecionada (Application Loopback).
    #[default]
    Application,
    /// Todo o áudio do sistema (fallback; a interface deve avisar o usuário).
    System,
}

/// Perfis de qualidade (S-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QualityProfile {
    Economico,
    #[default]
    Equilibrado,
    Qualidade,
    Competitivo,
    Personalizado,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality {
    /// Resolução de saída; a fonte é redimensionada mantendo a proporção.
    pub max_width: u32,
    pub max_height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
}

impl QualityProfile {
    /// Valores iniciais de cada perfil. O plano gratuito permite até 1920x1080 a 60 FPS.
    pub fn quality(self) -> Quality {
        match self {
            Self::Economico => Quality {
                max_width: 1280,
                max_height: 720,
                fps: 30,
                bitrate_kbps: 2500,
            },
            Self::Equilibrado => Quality {
                max_width: 1920,
                max_height: 1080,
                fps: 60,
                bitrate_kbps: 8000,
            },
            Self::Qualidade => Quality {
                max_width: 1920,
                max_height: 1080,
                fps: 60,
                bitrate_kbps: 12000,
            },
            Self::Competitivo => Quality {
                max_width: 1280,
                max_height: 720,
                fps: 60,
                bitrate_kbps: 5000,
            },
            Self::Personalizado => Self::Equilibrado.quality(),
        }
    }
}

impl Quality {
    pub const MAX_WIDTH: u32 = 1920;
    pub const MAX_HEIGHT: u32 = 1080;
    pub const MAX_FPS: u32 = 60;
    pub const MIN_BITRATE_KBPS: u32 = 500;
    pub const MAX_BITRATE_KBPS: u32 = 20000;

    /// Limita os valores ao intervalo suportado.
    pub fn clamped(self) -> Self {
        Self {
            max_width: self.max_width.clamp(320, Self::MAX_WIDTH),
            max_height: self.max_height.clamp(180, Self::MAX_HEIGHT),
            fps: self.fps.clamp(15, Self::MAX_FPS),
            bitrate_kbps: self
                .bitrate_kbps
                .clamp(Self::MIN_BITRATE_KBPS, Self::MAX_BITRATE_KBPS),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionConfig {
    pub source: SourceInfo,
    pub quality: Quality,
    pub audio: AudioMode,
    pub encoder: EncoderPreference,
    /// URL WebSocket do relay, ex.: `ws://127.0.0.1:8788/ws` ou `wss://relay.exemplo.com/ws`.
    pub relay_ws_url: String,
    /// Token de transmissor emitido pelo relay para a sala.
    pub publisher_token: String,
    /// Adaptação de bitrate/FPS durante a transmissão (S-001; competitivo = S-002).
    pub adaptation: Adaptation,
    /// Nome do transmissor na lista de participantes vista pelos espectadores.
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// Pipeline iniciado; valores efetivos após o redimensionamento.
    Started {
        encoder: String,
        width: u32,
        height: u32,
        fps: u32,
    },
    /// Conectado e autenticado no relay.
    Connected,
    /// Conexão com o relay perdida; tentando novamente.
    Reconnecting { reason: String },
    /// Aviso não fatal (ex.: áudio por processo indisponível, usando áudio do sistema).
    Warning { message: String },
    /// A janela capturada foi fechada ou o monitor desconectado. A sessão
    /// continua conectada (imagem congelada para os espectadores) esperando
    /// outra fonte por [`LiveChange::Video`]; sem nova fonte em
    /// [`SOURCE_LOST_GRACE`], ela encerra com [`SessionEvent::Stopped`].
    SourceLost,
    /// Captura e encoder reiniciados por [`LiveChange::Video`] (fonte,
    /// resolução ou FPS novos); valores efetivos após o redimensionamento.
    VideoRestarted {
        source: String,
        encoder: String,
        width: u32,
        height: u32,
        fps: u32,
    },
    /// Sessão encerrada (por [`StreamSession::stop`] ou por erro fatal).
    Stopped { reason: Option<String> },
}

/// Métricas da sessão (S-007), lidas pela interface sob demanda.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SessionStats {
    pub uptime: Duration,
    pub captured_fps: f32,
    pub encoded_fps: f32,
    /// Quadros descartados antes do encoder (fila cheia ou atraso).
    pub frames_dropped: u64,
    /// Quadros de vídeo descartados pelo transporte (rede lenta).
    pub network_dropped: u64,
    pub bitrate_kbps: f32,
    pub encode_ms_avg: f32,
    pub audio_active: bool,
    pub reconnects: u64,
    pub rtt_ms: Option<f32>,
    /// Bitrate-alvo atual definido pela adaptação (kbps); 0 = ainda não iniciado.
    pub target_bitrate_kbps: u32,
    /// FPS-alvo atual definido pela adaptação; 0 = ainda não iniciado.
    pub target_fps: u32,
    /// Áudio do aplicativo/sistema sendo capturado agora.
    pub game_audio: bool,
    pub microphone: MicStatus,
    /// `Some(false)` quando o encoder recusou mudar o bitrate em tempo real
    /// (mudanças de bitrate passam a exigir reinício); `None` = ainda não testado.
    pub live_bitrate: Option<bool>,
}

/// Estado do microfone na transmissão.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicStatus {
    #[default]
    Off,
    Starting,
    Active,
    Muted,
    /// Dispositivo ausente ou desconectado; a transmissão segue sem microfone.
    Unavailable,
}

/// Microfone (dispositivo de entrada) disponível.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioInputInfo {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Microfone escolhido para a transmissão (opt-in; desligado por padrão).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MicrophoneConfig {
    /// ID de [`list_audio_inputs`]; `None` = padrão do Windows.
    pub device_id: Option<String>,
    pub muted: bool,
}

/// Mudança pedida durante a transmissão ([`StreamSession::apply`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveChange {
    /// Bitrate novo (ao vivo; se o encoder recusar, reinicia o encoder).
    Bitrate(u32),
    /// Modo da adaptação (preferência de conteúdo), ao vivo.
    Adaptation(Adaptation),
    /// Fonte e qualidade novas: reinicia captura e encoder (breve congelamento
    /// da imagem), mantendo a conexão com a sala.
    Video {
        source: SourceInfo,
        quality: Quality,
    },
    /// Áudio do aplicativo/sistema, ao vivo.
    Audio(AudioMode),
    /// Liga, troca ou desliga (`None`) o microfone, ao vivo.
    Microphone(Option<MicrophoneConfig>),
    MicrophoneMuted(bool),
}

/// Tempo que uma sessão sem fonte (janela fechada, monitor desconectado)
/// espera o usuário escolher outra antes de encerrar.
pub const SOURCE_LOST_GRACE: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("recurso disponível apenas no Windows 10 2004 ou superior")]
    Unsupported,
    #[error("fonte indisponível: {0}")]
    SourceUnavailable(String),
    #[error("nenhum encoder H.264 disponível")]
    NoEncoder,
    #[error("falha no encoder: {0}")]
    Encoder(String),
    #[error("falha na captura: {0}")]
    Capture(String),
    #[error("falha no áudio: {0}")]
    Audio(String),
    #[error("configuração inválida: {0}")]
    InvalidConfig(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;

/// Callback de eventos; chamado de threads internas, nunca da thread da UI.
pub type EventSink = Box<dyn Fn(SessionEvent) + Send + Sync + 'static>;

/// Lista janelas e monitores capturáveis.
pub fn list_sources() -> Result<Vec<SourceInfo>> {
    platform::list_sources()
}

/// Lista encoders H.264 disponíveis, hardware primeiro.
pub fn list_encoders() -> Result<Vec<EncoderInfo>> {
    platform::list_encoders()
}

/// Lista os microfones disponíveis (vazia quando não há nenhum).
pub fn list_audio_inputs() -> Result<Vec<AudioInputInfo>> {
    platform::list_audio_inputs()
}

/// Uma transmissão ativa. Encerrada por [`StreamSession::stop`] ou ao ser descartada.
pub struct StreamSession {
    inner: platform::Session,
}

impl StreamSession {
    /// Inicia captura, codificação e envio. Retorna rapidamente; o progresso chega por `events`.
    pub fn start(config: SessionConfig, events: EventSink) -> Result<Self> {
        if config.publisher_token.is_empty() || config.relay_ws_url.is_empty() {
            return Err(CoreError::InvalidConfig("relay ou token ausente".into()));
        }
        let config = SessionConfig {
            quality: config.quality.clamped(),
            ..config
        };
        Ok(Self {
            inner: platform::Session::start(config, events)?,
        })
    }

    pub fn stats(&self) -> SessionStats {
        self.inner.stats()
    }

    /// Pede um keyframe imediato (ex.: novo espectador).
    pub fn request_keyframe(&self) {
        self.inner.request_keyframe();
    }

    /// Aplica uma mudança durante a transmissão. Não bloqueia: o pipeline
    /// aplica na própria thread e informa o resultado por eventos
    /// ([`SessionEvent::VideoRestarted`], [`SessionEvent::Warning`]).
    pub fn apply(&self, change: LiveChange) {
        let change = match change {
            LiveChange::Video { source, quality } => LiveChange::Video {
                source,
                quality: quality.clamped(),
            },
            LiveChange::Bitrate(kbps) => LiveChange::Bitrate(
                kbps.clamp(Quality::MIN_BITRATE_KBPS, Quality::MAX_BITRATE_KBPS),
            ),
            other => other,
        };
        self.inner.apply(change);
    }

    /// Encerra a sessão e libera captura, encoder e conexão.
    pub fn stop(self) {
        self.inner.stop();
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn list_sources() -> Result<Vec<SourceInfo>> {
        Err(CoreError::Unsupported)
    }

    pub fn list_encoders() -> Result<Vec<EncoderInfo>> {
        Err(CoreError::Unsupported)
    }

    pub fn list_audio_inputs() -> Result<Vec<AudioInputInfo>> {
        Err(CoreError::Unsupported)
    }

    pub struct Session;

    impl Session {
        pub fn start(_config: SessionConfig, _events: EventSink) -> Result<Self> {
            Err(CoreError::Unsupported)
        }
        pub fn stats(&self) -> SessionStats {
            SessionStats::default()
        }
        pub fn request_keyframe(&self) {}
        pub fn apply(&self, _change: LiveChange) {}
        pub fn stop(self) {}
    }
}

#[cfg(windows)]
mod platform;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_profiles_never_exceed_1080p60() {
        for p in [
            QualityProfile::Economico,
            QualityProfile::Equilibrado,
            QualityProfile::Qualidade,
            QualityProfile::Competitivo,
            QualityProfile::Personalizado,
        ] {
            let q = p.quality();
            assert_eq!(q, q.clamped(), "{p:?}");
        }
    }

    #[test]
    fn clamps_out_of_range_values() {
        let q = Quality {
            max_width: 7680,
            max_height: 4320,
            fps: 240,
            bitrate_kbps: 1,
        }
        .clamped();
        assert_eq!(
            q,
            Quality {
                max_width: 1920,
                max_height: 1080,
                fps: 60,
                bitrate_kbps: 500
            }
        );
    }
}
