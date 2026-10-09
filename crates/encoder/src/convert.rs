//! Conversão BGRA → NV12 com redimensionamento na GPU (`ID3D11VideoProcessor`).
//!
//! O quadro capturado nunca passa pela CPU: o processador de vídeo escreve
//! direto em uma textura NV12 que é entregue ao encoder. Sem processador de
//! vídeo (VM, Área de Trabalho Remota, driver básico), [`CpuConverter`] faz a
//! conversão na CPU como fallback.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
    D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_YCbCrA,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Media::MediaFoundation::{IMFMediaBuffer, MFCreateMemoryBuffer};
use windows::core::Interface;

use crate::EncoderError;
use crate::cpu_convert::{bgra_to_nv12, nv12_len};
use crate::sizing::letterbox;

/// Texturas NV12 em rodízio. Encoders assíncronos podem segurar a amostra de
/// entrada por alguns quadros; com modo de baixa latência há no máximo 1–2 em uso.
const RING_SIZE: usize = 4;

/// RGB de faixa completa (0–255): todos os bits zerados.
const COLOR_SPACE_RGB_FULL: u32 = 0;
/// YCbCr BT.709 em faixa de estúdio (16–235): `YCbCr_Matrix = 1` (bit 2) e
/// `Nominal_Range = 1` (bits 4–5), conforme `D3D11_VIDEO_PROCESSOR_COLOR_SPACE`.
const COLOR_SPACE_YCBCR_709_STUDIO: u32 = (1 << 2) | (1 << 4);

struct OutputSlot {
    texture: ID3D11Texture2D,
    view: ID3D11VideoProcessorOutputView,
}

pub(crate) struct VideoConverter {
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    outputs: Vec<OutputSlot>,
    next: usize,
    out_w: u32,
    out_h: u32,
    /// Dimensões da fonte para as quais os retângulos foram configurados.
    configured_src: (u32, u32),
    /// Cache da última view de entrada (o pool da captura reutiliza poucas texturas).
    input_views: Vec<(usize, ID3D11VideoProcessorInputView)>,
}

impl VideoConverter {
    pub fn new(
        device: &ID3D11Device,
        out_w: u32,
        out_h: u32,
        fps: u32,
    ) -> Result<Self, EncoderError> {
        let video_device: ID3D11VideoDevice = device
            .cast()
            .map_err(|e| EncoderError::win("ID3D11VideoDevice indisponível", e))?;
        // SAFETY: `device` é um dispositivo válido.
        let video_context: ID3D11VideoContext = unsafe { device.GetImmediateContext() }
            .map_err(|e| EncoderError::win("GetImmediateContext", e))?
            .cast()
            .map_err(|e| EncoderError::win("ID3D11VideoContext indisponível", e))?;

        // A entrada é descrita com o tamanho de saída: o processador aceita
        // fontes de outros tamanhos via retângulo de origem.
        let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            InputWidth: out_w,
            InputHeight: out_h,
            OutputFrameRate: DXGI_RATIONAL {
                Numerator: fps,
                Denominator: 1,
            },
            OutputWidth: out_w,
            OutputHeight: out_h,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        // SAFETY: descritor válido na pilha durante a chamada.
        let enumerator = unsafe { video_device.CreateVideoProcessorEnumerator(&content) }
            .map_err(|e| EncoderError::win("CreateVideoProcessorEnumerator", e))?;
        // SAFETY: enumerador recém-criado; índice 0 sempre existe.
        let processor = unsafe { video_device.CreateVideoProcessor(&enumerator, 0) }
            .map_err(|e| EncoderError::win("CreateVideoProcessor", e))?;

        let mut outputs = Vec::with_capacity(RING_SIZE);
        for _ in 0..RING_SIZE {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: out_w,
                Height: out_h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_NV12,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut texture = None;
            // SAFETY: descritor válido; ponteiro de saída local.
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
                .map_err(|e| EncoderError::win("CreateTexture2D NV12", e))?;
            let texture = texture.ok_or_else(|| EncoderError::Platform {
                context: "CreateTexture2D NV12",
                message: "textura nula".into(),
            })?;
            let view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };
            let mut view = None;
            // SAFETY: textura e enumerador válidos; descritor na pilha.
            unsafe {
                video_device.CreateVideoProcessorOutputView(
                    &texture,
                    &enumerator,
                    &view_desc,
                    Some(&mut view),
                )
            }
            .map_err(|e| EncoderError::win("CreateVideoProcessorOutputView", e))?;
            let view = view.ok_or_else(|| EncoderError::Platform {
                context: "CreateVideoProcessorOutputView",
                message: "view nula".into(),
            })?;
            outputs.push(OutputSlot { texture, view });
        }

        let converter = Self {
            video_device,
            video_context,
            enumerator,
            processor,
            outputs,
            next: 0,
            out_w,
            out_h,
            configured_src: (0, 0),
            input_views: Vec::new(),
        };
        converter.configure_static();
        Ok(converter)
    }

    fn configure_static(&self) {
        let black = D3D11_VIDEO_COLOR {
            Anonymous: D3D11_VIDEO_COLOR_0 {
                YCbCr: D3D11_VIDEO_COLOR_YCbCrA {
                    Y: 16.0 / 255.0,
                    Cb: 0.5,
                    Cr: 0.5,
                    A: 1.0,
                },
            },
        };
        let input_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
            _bitfield: COLOR_SPACE_RGB_FULL,
        };
        let output_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
            _bitfield: COLOR_SPACE_YCBCR_709_STUDIO,
        };
        let full = RECT {
            left: 0,
            top: 0,
            right: self.out_w as i32,
            bottom: self.out_h as i32,
        };
        // SAFETY: processador válido; ponteiros apontam para valores na pilha.
        unsafe {
            let vc = &self.video_context;
            vc.VideoProcessorSetStreamFrameFormat(
                &self.processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            vc.VideoProcessorSetStreamColorSpace(&self.processor, 0, &input_cs);
            vc.VideoProcessorSetOutputColorSpace(&self.processor, &output_cs);
            vc.VideoProcessorSetOutputBackgroundColor(&self.processor, true, &black);
            vc.VideoProcessorSetOutputTargetRect(&self.processor, true, Some(&full));
            // Desliga filtros automáticos do driver (custo e latência desnecessários).
            vc.VideoProcessorSetStreamAutoProcessingMode(&self.processor, 0, false);
        }
    }

    fn configure_rects(&mut self, src_w: u32, src_h: u32) {
        if self.configured_src == (src_w, src_h) {
            return;
        }
        let (x, y, w, h) = letterbox(src_w, src_h, self.out_w, self.out_h);
        let src = RECT {
            left: 0,
            top: 0,
            right: src_w as i32,
            bottom: src_h as i32,
        };
        let dst = RECT {
            left: x as i32,
            top: y as i32,
            right: (x + w) as i32,
            bottom: (y + h) as i32,
        };
        // SAFETY: processador válido; retângulos na pilha.
        unsafe {
            self.video_context.VideoProcessorSetStreamSourceRect(
                &self.processor,
                0,
                true,
                Some(&src),
            );
            self.video_context.VideoProcessorSetStreamDestRect(
                &self.processor,
                0,
                true,
                Some(&dst),
            );
        }
        self.configured_src = (src_w, src_h);
    }

    fn input_view(
        &mut self,
        texture: &ID3D11Texture2D,
    ) -> Result<ID3D11VideoProcessorInputView, EncoderError> {
        let key = texture.as_raw() as usize;
        if let Some((_, view)) = self.input_views.iter().find(|(k, _)| *k == key) {
            return Ok(view.clone());
        }
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut view = None;
        // SAFETY: textura e enumerador válidos; descritor na pilha.
        unsafe {
            self.video_device.CreateVideoProcessorInputView(
                texture,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )
        }
        .map_err(|e| EncoderError::win("CreateVideoProcessorInputView", e))?;
        let view = view.ok_or_else(|| EncoderError::Platform {
            context: "CreateVideoProcessorInputView",
            message: "view nula".into(),
        })?;
        // A view mantém a textura viva; guardamos poucas para não segurar o pool da captura.
        if self.input_views.len() >= 4 {
            self.input_views.remove(0);
        }
        self.input_views.push((key, view.clone()));
        Ok(view)
    }

    /// Converte a região `content_w`×`content_h` de `source` (BGRA) para a próxima
    /// textura NV12 do rodízio. O conteúdo pode ser menor que a textura quando a
    /// janela encolhe antes de o pool da captura ser recriado.
    pub fn convert(
        &mut self,
        source: &ID3D11Texture2D,
        content_w: u32,
        content_h: u32,
    ) -> Result<ID3D11Texture2D, EncoderError> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: textura válida; descritor de saída na pilha.
        unsafe { source.GetDesc(&mut desc) };
        let w = if content_w == 0 {
            desc.Width
        } else {
            content_w.min(desc.Width)
        };
        let h = if content_h == 0 {
            desc.Height
        } else {
            content_h.min(desc.Height)
        };
        self.configure_rects(w, h);
        let input = self.input_view(source)?;
        let slot = &self.outputs[self.next];
        self.next = (self.next + 1) % self.outputs.len();

        let stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            OutputIndex: 0,
            InputFrameOrField: 0,
            PastFrames: 0,
            FutureFrames: 0,
            ppPastSurfaces: std::ptr::null_mut(),
            pInputSurface: std::mem::ManuallyDrop::new(Some(input)),
            ppFutureSurfaces: std::ptr::null_mut(),
            ppPastSurfacesRight: std::ptr::null_mut(),
            pInputSurfaceRight: std::mem::ManuallyDrop::new(None),
            ppFutureSurfacesRight: std::ptr::null_mut(),
        };
        let mut streams = [stream];
        // SAFETY: processador, view de saída e view de entrada são válidos durante a chamada.
        let result = unsafe {
            self.video_context
                .VideoProcessorBlt(&self.processor, &slot.view, 0, &streams)
        };
        // Libera a referência que colocamos no ManuallyDrop.
        // SAFETY: o campo foi inicializado acima com `Some(input)` e não é usado depois.
        unsafe { std::mem::ManuallyDrop::drop(&mut streams[0].pInputSurface) };
        result.map_err(|e| EncoderError::win("VideoProcessorBlt", e))?;
        Ok(slot.texture.clone())
    }

    /// Esquece as views de entrada (ex.: o pool da captura foi recriado).
    pub fn reset_inputs(&mut self) {
        self.input_views.clear();
        self.configured_src = (0, 0);
    }
}

/// Fallback sem processador de vídeo D3D11: lê o quadro BGRA por uma textura
/// staging e converte para NV12 na CPU ([`crate::cpu_convert`]).
pub(crate) struct CpuConverter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    /// Textura staging no tamanho e formato da fonte (recriada se mudarem).
    staging: Option<(ID3D11Texture2D, u32, u32, DXGI_FORMAT)>,
    nv12: Vec<u8>,
    out_w: u32,
    out_h: u32,
}

impl CpuConverter {
    pub fn new(device: &ID3D11Device, out_w: u32, out_h: u32) -> Result<Self, EncoderError> {
        // SAFETY: `device` é um dispositivo válido.
        let context = unsafe { device.GetImmediateContext() }
            .map_err(|e| EncoderError::win("GetImmediateContext", e))?;
        Ok(Self {
            device: device.clone(),
            context,
            staging: None,
            nv12: vec![0; nv12_len(out_w, out_h)],
            out_w,
            out_h,
        })
    }

    fn staging_for(
        &mut self,
        desc: &D3D11_TEXTURE2D_DESC,
    ) -> Result<ID3D11Texture2D, EncoderError> {
        if let Some((tex, w, h, fmt)) = &self.staging
            && (*w, *h, *fmt) == (desc.Width, desc.Height, desc.Format)
        {
            return Ok(tex.clone());
        }
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: desc.Width,
            Height: desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: desc.Format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut texture = None;
        // SAFETY: descritor válido na pilha; saída local.
        unsafe {
            self.device
                .CreateTexture2D(&staging_desc, None, Some(&mut texture))
        }
        .map_err(|e| EncoderError::win("CreateTexture2D staging BGRA", e))?;
        let texture = texture.ok_or_else(|| EncoderError::Platform {
            context: "CreateTexture2D staging BGRA",
            message: "textura nula".into(),
        })?;
        self.staging = Some((texture.clone(), desc.Width, desc.Height, desc.Format));
        Ok(texture)
    }

    /// Converte a região `content_w`×`content_h` de `source` em um buffer NV12 de memória.
    pub fn convert(
        &mut self,
        source: &ID3D11Texture2D,
        content_w: u32,
        content_h: u32,
    ) -> Result<IMFMediaBuffer, EncoderError> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        // SAFETY: textura válida; descritor de saída na pilha.
        unsafe { source.GetDesc(&mut desc) };
        let w = if content_w == 0 {
            desc.Width
        } else {
            content_w.min(desc.Width)
        };
        let h = if content_h == 0 {
            desc.Height
        } else {
            content_h.min(desc.Height)
        };
        let staging = self.staging_for(&desc)?;
        let (out_w, out_h) = (self.out_w, self.out_h);
        let size = nv12_len(out_w, out_h);

        // SAFETY: cópia GPU→staging do mesmo tamanho e formato; o mapeamento é
        // lido somente dentro deste bloco (pitch × altura bytes) e desfeito antes de sair.
        unsafe {
            self.context.CopyResource(&staging, source);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| EncoderError::win("Map staging BGRA", e))?;
            let pitch = mapped.RowPitch as usize;
            let src =
                std::slice::from_raw_parts(mapped.pData as *const u8, pitch * desc.Height as usize);
            let ok = bgra_to_nv12(src, pitch, w, h, &mut self.nv12, out_w, out_h);
            self.context.Unmap(&staging, 0);
            if !ok {
                return Err(EncoderError::Platform {
                    context: "conversão BGRA→NV12 na CPU",
                    message: format!("tamanhos inválidos: fonte {w}x{h}, saída {out_w}x{out_h}"),
                });
            }
        }

        // SAFETY: buffer recém-criado com `size` bytes; travado só durante a cópia.
        unsafe {
            let buffer = MFCreateMemoryBuffer(size as u32)
                .map_err(|e| EncoderError::win("MFCreateMemoryBuffer", e))?;
            let mut dst = std::ptr::null_mut();
            buffer
                .Lock(&mut dst, None, None)
                .map_err(|e| EncoderError::win("Lock", e))?;
            std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), dst, size);
            let _ = buffer.Unlock();
            buffer
                .SetCurrentLength(size as u32)
                .map_err(|e| EncoderError::win("SetCurrentLength", e))?;
            Ok(buffer)
        }
    }
}
