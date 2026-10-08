//! Sessão de captura Windows Graphics Capture.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{IInspectable, Interface};

use crate::pacing::FramePacer;
use crate::slot::LatestSlot;
use crate::{CaptureError, CaptureOptions, CaptureTarget, D3dDevice};

/// Buffers do pool: um em uso pelo encoder e um sendo preenchido.
/// Três buffers: o pipeline guarda o último quadro para repeti-lo quando a fonte
/// fica parada, e o WGC ainda tem dois livres para quadros novos.
const POOL_BUFFERS: i32 = 3;
const PIXEL_FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

/// Um quadro capturado, ainda na GPU.
///
/// Mantém o `Direct3D11CaptureFrame` vivo até ser descartado: o buffer do pool
/// só volta a ser usado depois disso, então o encoder pode ler a textura sem cópia.
pub struct CapturedFrame {
    pub texture: ID3D11Texture2D,
    /// Região válida da textura (o conteúdo pode ser menor que a textura).
    pub width: u32,
    pub height: u32,
    /// `SystemRelativeTime` do quadro: relógio QPC em unidades de 100 ns.
    pub time_hns: i64,
    frame: Direct3D11CaptureFrame,
}

// SAFETY: a textura pertence a um dispositivo D3D11 com proteção multithread e o
// quadro WinRT é ágil; o pipeline usa cada quadro em uma única thread por vez.
unsafe impl Send for CapturedFrame {}

impl Drop for CapturedFrame {
    fn drop(&mut self) {
        // Devolve o buffer ao pool imediatamente.
        let _ = self.frame.Close();
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CaptureStats {
    /// Quadros entregues pelo WGC.
    pub arrived: u64,
    /// Quadros descartados pelo controle de ritmo (acima da taxa-alvo).
    pub paced_out: u64,
    /// Quadros substituídos antes de o encoder pegá-los (encoder ocupado).
    pub replaced: u64,
}

struct Shared {
    slot: Arc<LatestSlot<CapturedFrame>>,
    pacer: Mutex<FramePacer>,
    size: Mutex<SizeInt32>,
    device: D3dDevice,
    arrived: AtomicU64,
    paced_out: AtomicU64,
    replaced: AtomicU64,
    closed: AtomicBool,
}

/// Captura em andamento. Encerrada por [`Capturer::stop`] ou ao ser descartada.
pub struct Capturer {
    shared: Arc<Shared>,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    item: GraphicsCaptureItem,
    frame_token: i64,
    closed_token: i64,
    stopped: bool,
}

// SAFETY: os objetos WinRT de captura são ágeis (free-threaded pool) e o
// `Capturer` só é usado pela thread do pipeline.
unsafe impl Send for Capturer {}

impl Capturer {
    /// Inicia a captura da fonte escolhida pelo usuário.
    ///
    /// `on_closed` é chamado (de outra thread) quando a janela é fechada ou o
    /// monitor desconectado.
    pub fn start(
        device: &D3dDevice,
        target: CaptureTarget,
        options: CaptureOptions,
        slot: Arc<LatestSlot<CapturedFrame>>,
        on_closed: Box<dyn Fn() + Send + Sync>,
    ) -> Result<Self, CaptureError> {
        // SAFETY: inicialização WinRT da thread atual; já inicializada é aceitável.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err(CaptureError::Unsupported);
        }
        let item = create_item(target)?;
        let size = item
            .Size()
            .map_err(|e| CaptureError::win("GraphicsCaptureItem.Size", e))?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &device.winrt,
            PIXEL_FORMAT,
            POOL_BUFFERS,
            size,
        )
        .map_err(|e| CaptureError::win("CreateFreeThreaded", e))?;
        let session = pool
            .CreateCaptureSession(&item)
            .map_err(|e| CaptureError::win("CreateCaptureSession", e))?;
        // Recursos opcionais: disponíveis a partir de certas versões do Windows.
        if let Err(err) = session.SetIsCursorCaptureEnabled(options.cursor) {
            tracing::debug!(%err, "IsCursorCaptureEnabled indisponível");
        }
        if let Err(err) = session.SetIsBorderRequired(false) {
            tracing::debug!(%err, "IsBorderRequired indisponível; a borda amarela do Windows será exibida");
        }

        let shared = Arc::new(Shared {
            slot,
            pacer: Mutex::new(FramePacer::new(options.fps)),
            size: Mutex::new(size),
            device: device.clone(),
            arrived: AtomicU64::new(0),
            paced_out: AtomicU64::new(0),
            replaced: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        });

        let handler_shared = Arc::clone(&shared);
        let frame_token = pool
            .FrameArrived(
                &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(
                    move |sender, _| {
                        if let Ok(pool) = sender.ok() {
                            on_frame(&handler_shared, pool);
                        }
                        Ok(())
                    },
                ),
            )
            .map_err(|e| CaptureError::win("FrameArrived", e))?;

        let closed_shared = Arc::clone(&shared);
        let closed_token = item
            .Closed(
                &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                    if !closed_shared.closed.swap(true, Ordering::SeqCst) {
                        on_closed();
                    }
                    Ok(())
                }),
            )
            .map_err(|e| CaptureError::win("Closed", e))?;

        session
            .StartCapture()
            .map_err(|e| CaptureError::win("StartCapture", e))?;
        Ok(Self {
            shared,
            pool,
            session,
            item,
            frame_token,
            closed_token,
            stopped: false,
        })
    }

    pub fn stats(&self) -> CaptureStats {
        CaptureStats {
            arrived: self.shared.arrived.load(Ordering::Relaxed),
            paced_out: self.shared.paced_out.load(Ordering::Relaxed),
            replaced: self.shared.replaced.load(Ordering::Relaxed),
        }
    }

    /// A fonte foi fechada.
    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    /// Para de receber quadros sem descartar o objeto (idempotente).
    pub fn halt(&mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        let _ = self.pool.RemoveFrameArrived(self.frame_token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

impl Drop for Capturer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn create_item(target: CaptureTarget) -> Result<GraphicsCaptureItem, CaptureError> {
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|e| CaptureError::win("IGraphicsCaptureItemInterop", e))?;
    // SAFETY: HWND/HMONITOR vêm da enumeração; se não forem mais válidos, a chamada falha.
    let result = unsafe {
        match target {
            CaptureTarget::Window(h) => {
                interop.CreateForWindow::<GraphicsCaptureItem>(HWND(h as *mut _))
            }
            CaptureTarget::Monitor(h) => {
                interop.CreateForMonitor::<GraphicsCaptureItem>(HMONITOR(h as *mut _))
            }
        }
    };
    result.map_err(|_| CaptureError::SourceGone)
}

fn on_frame(shared: &Shared, pool: &Direct3D11CaptureFramePool) {
    let Ok(frame) = pool.TryGetNextFrame() else {
        return;
    };
    shared.arrived.fetch_add(1, Ordering::Relaxed);

    let Ok(content) = frame.ContentSize() else {
        return;
    };
    {
        let mut size = shared.size.lock().unwrap_or_else(|p| p.into_inner());
        if content.Width != size.Width || content.Height != size.Height {
            // A fonte mudou de tamanho: recria o pool e descarta este quadro.
            *size = content;
            drop(size);
            let _ = frame.Close();
            if let Err(err) =
                pool.Recreate(&shared.device.winrt, PIXEL_FORMAT, POOL_BUFFERS, content)
            {
                tracing::warn!(%err, "falha ao recriar o pool de captura");
            }
            return;
        }
    }

    let time_hns = frame.SystemRelativeTime().map(|t| t.Duration).unwrap_or(0);
    let accepted = shared
        .pacer
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .accept(time_hns);
    if !accepted {
        shared.paced_out.fetch_add(1, Ordering::Relaxed);
        let _ = frame.Close();
        return;
    }

    let texture = frame
        .Surface()
        .and_then(|s| s.cast::<IDirect3DDxgiInterfaceAccess>())
        // SAFETY: a superfície do quadro WGC expõe a textura D3D11 subjacente.
        .and_then(|access| unsafe { access.GetInterface::<ID3D11Texture2D>() });
    let Ok(texture) = texture else {
        let _ = frame.Close();
        return;
    };
    let captured = CapturedFrame {
        texture,
        width: content.Width.max(0) as u32,
        height: content.Height.max(0) as u32,
        time_hns,
        frame,
    };
    if shared.slot.put(captured).is_some() {
        shared.replaced.fetch_add(1, Ordering::Relaxed);
    }
}
