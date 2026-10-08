//! Dispositivo Direct3D 11 compartilhado entre captura, conversão e encoder.

use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0,
    D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_FLAG, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11Multithread,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::core::Interface;

use crate::CaptureError;

/// Dispositivo D3D11 com suporte a BGRA (WGC) e vídeo (conversão/encoder),
/// com proteção multithread ligada: a captura entrega quadros em uma thread do
/// pool e o encoder os consome em outra.
#[derive(Clone)]
pub struct D3dDevice {
    pub device: ID3D11Device,
    pub(crate) winrt: IDirect3DDevice,
}

// SAFETY: o dispositivo é criado com proteção multithread (`ID3D11Multithread`),
// e `IDirect3DDevice` é um objeto WinRT ágil.
unsafe impl Send for D3dDevice {}
// SAFETY: idem; o contexto imediato é serializado pela proteção multithread.
unsafe impl Sync for D3dDevice {}

impl D3dDevice {
    pub fn new() -> Result<Self, CaptureError> {
        let device = create(D3D11_CREATE_DEVICE_FLAG(
            D3D11_CREATE_DEVICE_BGRA_SUPPORT.0 | D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0,
        ))
        .or_else(|err| {
            tracing::warn!(%err, "D3D11 sem VIDEO_SUPPORT; tentando sem");
            create(D3D11_CREATE_DEVICE_BGRA_SUPPORT)
        })?;
        if let Ok(mt) = device.cast::<ID3D11Multithread>() {
            // SAFETY: interface válida do dispositivo recém-criado.
            unsafe {
                let _ = mt.SetMultithreadProtected(true);
            }
        }
        let dxgi: IDXGIDevice = device
            .cast()
            .map_err(|e| CaptureError::win("IDXGIDevice", e))?;
        // SAFETY: `dxgi` é o dispositivo DXGI do D3D11 recém-criado.
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .map_err(|e| CaptureError::win("CreateDirect3D11DeviceFromDXGIDevice", e))?;
        let winrt: IDirect3DDevice = inspectable
            .cast()
            .map_err(|e| CaptureError::win("IDirect3DDevice", e))?;
        Ok(Self { device, winrt })
    }

    /// Verdadeiro se o driver removeu ou reiniciou o dispositivo.
    pub fn is_lost(&self) -> bool {
        // SAFETY: consulta simples de estado do dispositivo.
        unsafe { self.device.GetDeviceRemovedReason() }.is_err()
    }
}

fn create(flags: D3D11_CREATE_DEVICE_FLAG) -> Result<ID3D11Device, CaptureError> {
    let levels = [
        D3D_FEATURE_LEVEL_11_1,
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_1,
    ];
    let mut device = None;
    // SAFETY: parâmetros válidos; saídas locais.
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            flags,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .map_err(|e| CaptureError::win("D3D11CreateDevice", e))?;
    device.ok_or(CaptureError::Platform {
        context: "D3D11CreateDevice",
        message: "dispositivo nulo".into(),
    })
}
