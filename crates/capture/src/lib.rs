//! Captura de janelas e monitores para o Vellcast.
//!
//! No Windows usa o **Windows Graphics Capture** (WGC): o usuário escolhe
//! explicitamente a janela ou o monitor, sem injeção em processos nem hooks.
//! Os quadros chegam como texturas Direct3D 11 e seguem na GPU até o encoder.
//!
//! [`pacing`], [`slot`] e [`heuristics`] são lógica pura, testada em qualquer plataforma.

pub mod heuristics;
pub mod pacing;
pub mod slot;

#[cfg(windows)]
pub mod clock;
#[cfg(windows)]
mod device;
#[cfg(windows)]
mod enumerate;
#[cfg(windows)]
mod wgc;

#[cfg(windows)]
pub use device::D3dDevice;
#[cfg(windows)]
pub use enumerate::list_sources;
#[cfg(windows)]
pub use wgc::{CaptureStats, CapturedFrame, Capturer};

/// O que capturar. Os valores são HWND/HMONITOR do Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureTarget {
    Window(isize),
    Monitor(isize),
}

/// Fonte capturável apresentada ao usuário.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    pub target: CaptureTarget,
    pub title: String,
    pub process_name: Option<String>,
    pub process_id: Option<u32>,
    pub width: u32,
    pub height: u32,
    pub likely_game: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureOptions {
    /// Taxa máxima de quadros entregues ao encoder.
    pub fps: u32,
    /// Desenhar o cursor (quando o sistema permitir desligar).
    pub cursor: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("captura disponível apenas no Windows 10 2004 ou superior")]
    Unsupported,
    #[error("a fonte não está mais disponível")]
    SourceGone,
    #[error("o dispositivo gráfico foi removido ou reiniciado")]
    DeviceLost,
    #[error("{context}: {message}")]
    Platform {
        context: &'static str,
        message: String,
    },
}

#[cfg(windows)]
impl CaptureError {
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
pub fn list_sources() -> Result<Vec<SourceDescriptor>, CaptureError> {
    Err(CaptureError::Unsupported)
}
