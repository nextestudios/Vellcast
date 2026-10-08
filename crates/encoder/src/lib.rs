//! Codificação H.264 para o Vellcast.
//!
//! - [`annexb`] e [`sizing`]: lógica pura, testável em qualquer plataforma.
//! - No Windows: encoders Media Foundation (hardware primeiro, software Microsoft
//!   como último recurso), alimentados por texturas Direct3D 11 convertidas para
//!   NV12 na própria GPU (`ID3D11VideoProcessor`), sem leitura pela CPU no caminho
//!   de hardware.

pub mod annexb;
pub mod sizing;

#[cfg(windows)]
mod convert;
#[cfg(windows)]
mod mf;

#[cfg(windows)]
pub use mf::{H264Encoder, init_thread, list_encoders};

/// Fabricante do encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Microsoft,
    Other,
}

impl Vendor {
    /// Interpreta `MFT_ENUM_HARDWARE_VENDOR_ID_Attribute` (ex.: `VEN_10DE`) e o nome amigável.
    pub fn detect(vendor_id: Option<&str>, name: &str) -> Self {
        let id = vendor_id.unwrap_or_default().to_ascii_uppercase();
        let name_lc = name.to_ascii_lowercase();
        if id.contains("10DE") || name_lc.contains("nvidia") {
            Self::Nvidia
        } else if id.contains("1002") || id.contains("1022") || name_lc.contains("amd") {
            Self::Amd
        } else if id.contains("8086") || name_lc.contains("intel") {
            Self::Intel
        } else if name_lc.contains("microsoft") {
            Self::Microsoft
        } else {
            Self::Other
        }
    }
}

/// Encoder H.264 disponível no sistema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderDescriptor {
    pub name: String,
    pub vendor: Vendor,
    pub hardware: bool,
}

/// Parâmetros do encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderConfig {
    /// Dimensões de saída (pares).
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    /// Intervalo entre keyframes automáticos, em segundos.
    pub keyframe_interval_s: u32,
    /// Nome do encoder preferido; `None` = automático (hardware primeiro).
    pub preferred: Option<String>,
}

/// Uma unidade de acesso codificada, já normalizada para o protocolo
/// (Annex B, sem AUD, SPS/PPS antes de todo IDR).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedFrame {
    pub data: Vec<u8>,
    /// Timestamp de apresentação em unidades de 100 ns (o mesmo do quadro de entrada).
    pub pts_hns: i64,
    pub keyframe: bool,
}

/// Resultado do envio de um quadro ao encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    Accepted,
    /// O encoder ainda não pediu mais entrada; o quadro foi descartado.
    Busy,
}

#[derive(Debug, thiserror::Error)]
pub enum EncoderError {
    #[error("codificação disponível apenas no Windows")]
    Unsupported,
    #[error("nenhum encoder H.264 disponível")]
    NoEncoder,
    #[error("o dispositivo gráfico foi removido ou reiniciado")]
    DeviceLost,
    #[error("{context}: {message}")]
    Platform {
        context: &'static str,
        message: String,
    },
}

#[cfg(windows)]
impl EncoderError {
    pub(crate) fn win(context: &'static str, err: windows::core::Error) -> Self {
        use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET};
        let code = err.code();
        if code == DXGI_ERROR_DEVICE_REMOVED || code == DXGI_ERROR_DEVICE_RESET {
            Self::DeviceLost
        } else {
            Self::Platform {
                context,
                message: err.to_string(),
            }
        }
    }
}

#[cfg(not(windows))]
pub fn list_encoders() -> Result<Vec<EncoderDescriptor>, EncoderError> {
    Err(EncoderError::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_vendors() {
        assert_eq!(
            Vendor::detect(Some("VEN_10DE"), "NVIDIA H.264 Encoder MFT"),
            Vendor::Nvidia
        );
        assert_eq!(
            Vendor::detect(Some("VEN_1002"), "AMDh264Encoder"),
            Vendor::Amd
        );
        assert_eq!(
            Vendor::detect(
                Some("VEN_8086"),
                "Intel® Quick Sync Video H.264 Encoder MFT"
            ),
            Vendor::Intel
        );
        assert_eq!(
            Vendor::detect(None, "H264 Encoder MFT (Microsoft)"),
            Vendor::Microsoft
        );
        assert_eq!(Vendor::detect(None, "Outro"), Vendor::Other);
    }
}
