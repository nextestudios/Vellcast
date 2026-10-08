//! Encoder H.264 via Media Foundation Transforms (MFT).
//!
//! Ordem de preferência: encoders de hardware (NVIDIA, AMD, Intel) e, como
//! último recurso, o encoder de software da Microsoft — assim o MVP funciona
//! em qualquer PC, com o aviso de que o caminho de software usa CPU.
//!
//! Encoders de hardware são assíncronos: o MFT emite `METransformNeedInput` e
//! `METransformHaveOutput`. O encoder de software é síncrono. Os dois modelos
//! são tratados aqui sem bloquear: o chamador alterna entre [`H264Encoder::submit`]
//! e [`H264Encoder::poll`].

use std::sync::Once;

use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode,
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize,
    CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVLowLatencyMode, ICodecAPI, IMF2DBuffer,
    IMFActivate, IMFDXGIDeviceManager, IMFMediaEventGenerator, IMFMediaType, IMFSample,
    IMFTransform, METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE,
    MF_E_NOTACCEPTING, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
    MF_EVENT_FLAG_NO_WAIT, MF_LOW_LATENCY, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_MPEG2_PROFILE,
    MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK,
    MF_VERSION, MFCreateDXGIDeviceManager, MFCreateDXGISurfaceBuffer, MFCreateMediaType,
    MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSTARTUP_LITE,
    MFSampleExtension_CleanPoint, MFStartup, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG,
    MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
    MFT_ENUM_HARDWARE_VENDOR_ID_Attribute, MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_COMMAND_FLUSH,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM,
    MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
    MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
    MFVideoInterlace_Progressive, eAVEncCommonRateControlMode_CBR, eAVEncH264VProfile_High,
    eAVEncH264VProfile_Main,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};
use windows::core::{GUID, Interface, PWSTR};

use crate::annexb::{self, ParameterSets};
use crate::convert::VideoConverter;
use crate::sizing::{bitrate_bps, frame_duration_hns, gop_frames};
use crate::{EncodedFrame, EncoderConfig, EncoderDescriptor, EncoderError, SubmitOutcome, Vendor};

/// Tamanho mínimo do buffer de saída quando o MFT não fornece amostras.
const MIN_OUTPUT_BUFFER: u32 = 2 * 1024 * 1024;

static MF_STARTUP: Once = Once::new();

/// Inicializa COM (MTA) na thread atual e o Media Foundation no processo.
///
/// Deve ser chamada em toda thread que usa o encoder. Ignora o caso em que a
/// thread já está em outro modo de apartment.
pub fn init_thread() {
    // SAFETY: inicialização de COM na thread atual; o retorno é apenas informativo.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    MF_STARTUP.call_once(|| {
        // SAFETY: chamada única por processo; MFShutdown não é chamado (o processo encerra).
        if let Err(err) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) } {
            tracing::error!(%err, "MFStartup falhou");
        }
    });
}

struct Candidate {
    descriptor: EncoderDescriptor,
    activate: IMFActivate,
}

fn read_string(activate: &IMFActivate, key: &GUID) -> Option<String> {
    let mut value = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: ponteiros de saída locais; a string alocada é liberada com CoTaskMemFree.
    unsafe {
        activate
            .GetAllocatedString(key, &mut value, &mut len)
            .ok()?;
        let s = value.to_string().ok();
        CoTaskMemFree(Some(value.0 as *const _));
        s
    }
}

fn enumerate(flags: MFT_ENUM_FLAG, hardware: bool) -> Vec<Candidate> {
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut array: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: ponteiros de saída locais; o array é liberado com CoTaskMemFree abaixo.
    let ok = unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
            None,
            Some(&output),
            &mut array,
            &mut count,
        )
    };
    if ok.is_err() || array.is_null() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        // SAFETY: `array` tem `count` elementos inicializados pelo MFTEnumEx; cada
        // referência é movida para fora exatamente uma vez.
        let item = unsafe { std::ptr::read(array.add(i)) };
        let Some(activate) = item else { continue };
        let name = read_string(&activate, &MFT_FRIENDLY_NAME_Attribute)
            .unwrap_or_else(|| "H.264 Encoder".into());
        let vendor_id = read_string(&activate, &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute);
        let vendor = Vendor::detect(vendor_id.as_deref(), &name);
        out.push(Candidate {
            descriptor: EncoderDescriptor {
                name,
                vendor,
                hardware,
            },
            activate,
        });
    }
    // SAFETY: o array foi alocado pelo MFTEnumEx; os elementos já foram movidos.
    unsafe { CoTaskMemFree(Some(array as *const _)) };
    out
}

fn candidates() -> Vec<Candidate> {
    init_thread();
    let mut all = enumerate(
        MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0),
        true,
    );
    all.extend(enumerate(
        MFT_ENUM_FLAG(MFT_ENUM_FLAG_SYNCMFT.0 | MFT_ENUM_FLAG_SORTANDFILTER.0),
        false,
    ));
    all
}

/// Lista os encoders H.264 do sistema, hardware primeiro.
pub fn list_encoders() -> Result<Vec<EncoderDescriptor>, EncoderError> {
    Ok(candidates().into_iter().map(|c| c.descriptor).collect())
}

fn variant_u32(value: u32) -> VARIANT {
    let mut v = VARIANT::default();
    // SAFETY: escrita de um VARIANT do tipo VT_UI4 em memória própria.
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_UI4;
        inner.Anonymous.ulVal = value;
    }
    v
}

fn variant_bool(value: bool) -> VARIANT {
    let mut v = VARIANT::default();
    // SAFETY: escrita de um VARIANT do tipo VT_BOOL em memória própria.
    unsafe {
        let inner = &mut *v.Anonymous.Anonymous;
        inner.vt = VT_BOOL;
        inner.Anonymous.boolVal =
            windows::Win32::Foundation::VARIANT_BOOL(if value { -1 } else { 0 });
    }
    v
}

fn pack(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

fn win(context: &'static str) -> impl FnOnce(windows::core::Error) -> EncoderError {
    move |e| EncoderError::win(context, e)
}

/// Caminho de entrada do encoder.
enum InputPath {
    /// Texturas NV12 entregues direto (encoder com suporte a Direct3D 11).
    Gpu { _manager: IMFDXGIDeviceManager },
    /// Encoder de software sem suporte a D3D: cópia NV12 para memória (somente fallback).
    Cpu {
        staging: ID3D11Texture2D,
        context: ID3D11DeviceContext,
    },
}

pub struct H264Encoder {
    descriptor: EncoderDescriptor,
    transform: IMFTransform,
    events: Option<IMFMediaEventGenerator>,
    codec_api: Option<ICodecAPI>,
    converter: VideoConverter,
    input: InputPath,
    config: EncoderConfig,
    params: ParameterSets,
    /// Créditos de entrada do modelo assíncrono (`METransformNeedInput`).
    need_input: u32,
    provides_samples: bool,
    output_buffer_size: u32,
    frame_duration: i64,
}

// SAFETY: as interfaces COM usadas são criadas em MTA e o encoder é usado por
// uma única thread por vez (o pipeline o move para a thread de codificação).
unsafe impl Send for H264Encoder {}

impl H264Encoder {
    /// Cria o encoder usando o mesmo dispositivo Direct3D 11 da captura.
    ///
    /// Tenta os candidatos em ordem (preferido, hardware, software) até um aceitar
    /// o dispositivo e os tipos de mídia.
    pub fn new(device: &ID3D11Device, config: EncoderConfig) -> Result<Self, EncoderError> {
        init_thread();
        let mut list = candidates();
        if let Some(name) = &config.preferred {
            list.sort_by_key(|c| c.descriptor.name != *name);
        }
        if list.is_empty() {
            return Err(EncoderError::NoEncoder);
        }
        let mut last_error = EncoderError::NoEncoder;
        for candidate in list {
            match Self::try_create(device, &config, candidate) {
                Ok(encoder) => return Ok(encoder),
                Err(EncoderError::DeviceLost) => return Err(EncoderError::DeviceLost),
                Err(err) => {
                    tracing::warn!(%err, "encoder recusado; tentando o próximo");
                    last_error = err;
                }
            }
        }
        Err(last_error)
    }

    fn try_create(
        device: &ID3D11Device,
        config: &EncoderConfig,
        candidate: Candidate,
    ) -> Result<Self, EncoderError> {
        let Candidate {
            descriptor,
            activate,
        } = candidate;
        // SAFETY: ativação de um MFT enumerado pelo sistema.
        let transform: IMFTransform =
            unsafe { activate.ActivateObject() }.map_err(win("ActivateObject"))?;

        // SAFETY: atributos do MFT recém-criado.
        let attributes = unsafe { transform.GetAttributes() }.ok();
        let is_async = attributes
            .as_ref()
            // SAFETY: leitura de atributo; ausência significa MFT síncrono.
            .and_then(|a| unsafe { a.GetUINT32(&MF_TRANSFORM_ASYNC) }.ok())
            == Some(1);
        if let Some(a) = &attributes {
            // SAFETY: escrita de atributos documentados no MFT.
            unsafe {
                if is_async {
                    a.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
                        .map_err(win("MF_TRANSFORM_ASYNC_UNLOCK"))?;
                }
                let _ = a.SetUINT32(&MF_LOW_LATENCY, 1);
            }
        }

        // Dispositivo compartilhado com a captura: o encoder lê as texturas NV12 direto.
        let mut token = 0u32;
        let mut manager = None;
        // SAFETY: ponteiros de saída locais.
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }
            .map_err(win("MFCreateDXGIDeviceManager"))?;
        let manager = manager.ok_or(EncoderError::Platform {
            context: "MFCreateDXGIDeviceManager",
            message: "nulo".into(),
        })?;
        // SAFETY: `device` é válido e tem proteção multithread ativada pela captura.
        unsafe { manager.ResetDevice(device, token) }.map_err(win("ResetDevice"))?;
        // SAFETY: o parâmetro é um ponteiro de interface válido durante a chamada; o MFT faz AddRef.
        let d3d_ok = unsafe {
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
        }
        .is_ok();

        let input = if d3d_ok {
            InputPath::Gpu { _manager: manager }
        } else if descriptor.hardware {
            // Encoder de hardware que não aceita este dispositivo (ex.: outra GPU).
            return Err(EncoderError::Platform {
                context: "MFT_MESSAGE_SET_D3D_MANAGER",
                message: format!("{} não aceita o dispositivo Direct3D", descriptor.name),
            });
        } else {
            tracing::warn!(encoder = %descriptor.name, "encoder de software sem Direct3D: usando cópia pela CPU");
            InputPath::Cpu {
                staging: create_staging(device, config.width, config.height)?,
                context: immediate_context(device)?,
            }
        };

        let codec_api: Option<ICodecAPI> = transform.cast().ok();
        let gop = gop_frames(config.fps, config.keyframe_interval_s);
        if let Some(api) = &codec_api {
            // Melhor esforço: nem todo encoder aceita todas as propriedades.
            // SAFETY: GUIDs e VARIANTs válidos durante as chamadas.
            unsafe {
                let _ = api.SetValue(&CODECAPI_AVLowLatencyMode, &variant_bool(true));
                let _ = api.SetValue(
                    &CODECAPI_AVEncCommonRateControlMode,
                    &variant_u32(eAVEncCommonRateControlMode_CBR.0 as u32),
                );
                let _ = api.SetValue(
                    &CODECAPI_AVEncCommonMeanBitRate,
                    &variant_u32(bitrate_bps(config.bitrate_kbps)),
                );
                let _ = api.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(gop));
                let _ = api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0));
            }
        }

        let output_type = h264_type(config, true)?;
        // SAFETY: tipo de mídia válido.
        if unsafe { transform.SetOutputType(0, &output_type, 0) }.is_err() {
            // Alguns encoders não oferecem High; Main também evita B-frames por padrão.
            let fallback = h264_type(config, false)?;
            // SAFETY: tipo de mídia válido.
            unsafe { transform.SetOutputType(0, &fallback, 0) }.map_err(win("SetOutputType"))?;
        }
        let input_type = nv12_type(&transform, config)?;
        // SAFETY: tipo de mídia válido.
        unsafe { transform.SetInputType(0, &input_type, 0) }.map_err(win("SetInputType"))?;

        // SAFETY: consulta de informação do fluxo de saída.
        let info =
            unsafe { transform.GetOutputStreamInfo(0) }.map_err(win("GetOutputStreamInfo"))?;
        let provides_samples = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32) != 0;

        let mut params = ParameterSets::default();
        // SAFETY: tipo atual do fluxo de saída já configurado.
        if let Ok(current) = unsafe { transform.GetOutputCurrentType(0) } {
            read_sequence_header(&current, &mut params);
        }

        // SAFETY: mensagens de início de fluxo documentadas.
        unsafe {
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(win("BEGIN_STREAMING"))?;
            transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(win("START_OF_STREAM"))?;
        }

        let events = if is_async {
            Some(
                transform
                    .cast::<IMFMediaEventGenerator>()
                    .map_err(win("IMFMediaEventGenerator"))?,
            )
        } else {
            None
        };
        let converter = VideoConverter::new(device, config.width, config.height, config.fps)?;

        tracing::info!(encoder = %descriptor.name, hardware = descriptor.hardware, is_async, gpu_input = d3d_ok, "encoder H.264 pronto");
        Ok(Self {
            descriptor,
            transform,
            events,
            codec_api,
            converter,
            input,
            config: config.clone(),
            params,
            need_input: 0,
            provides_samples,
            output_buffer_size: info.cbSize.max(MIN_OUTPUT_BUFFER),
            frame_duration: frame_duration_hns(config.fps),
        })
    }

    pub fn descriptor(&self) -> &EncoderDescriptor {
        &self.descriptor
    }

    pub fn config(&self) -> &EncoderConfig {
        &self.config
    }

    /// Codec string WebCodecs, disponível após o primeiro SPS.
    pub fn codec_string(&self) -> Option<String> {
        self.params.codec_string()
    }

    /// Esquece views de entrada em cache (o pool da captura foi recriado).
    pub fn reset_inputs(&mut self) {
        self.converter.reset_inputs();
    }

    /// Indica se o encoder aceita um novo quadro agora (sempre verdadeiro no modelo síncrono).
    pub fn ready_for_input(&self) -> bool {
        self.events.is_none() || self.need_input > 0
    }

    /// Converte e envia um quadro BGRA (região `width`×`height` da textura). Não bloqueia.
    #[allow(clippy::too_many_arguments)]
    pub fn submit(
        &mut self,
        source: &ID3D11Texture2D,
        width: u32,
        height: u32,
        pts_hns: i64,
        force_keyframe: bool,
        out: &mut Vec<EncodedFrame>,
    ) -> Result<SubmitOutcome, EncoderError> {
        self.pump(out)?;
        if !self.ready_for_input() {
            return Ok(SubmitOutcome::Busy);
        }
        if force_keyframe {
            if let Some(api) = &self.codec_api {
                // SAFETY: GUID e VARIANT válidos durante a chamada.
                let _ = unsafe { api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &variant_u32(1)) };
            }
        }
        let nv12 = self.converter.convert(source, width, height)?;
        let sample = self.make_sample(&nv12, pts_hns)?;
        // SAFETY: amostra válida para o fluxo 0.
        match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
            Ok(()) => {}
            Err(e) if e.code() == MF_E_NOTACCEPTING => {
                self.drain_sync(out)?;
                // SAFETY: idem.
                if unsafe { self.transform.ProcessInput(0, &sample, 0) }.is_err() {
                    return Ok(SubmitOutcome::Busy);
                }
            }
            Err(e) => return Err(EncoderError::win("ProcessInput", e)),
        }
        if self.events.is_some() {
            self.need_input = self.need_input.saturating_sub(1);
        } else {
            self.drain_sync(out)?;
        }
        Ok(SubmitOutcome::Accepted)
    }

    /// Coleta quadros codificados prontos, sem bloquear.
    pub fn poll(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), EncoderError> {
        self.pump(out)
    }

    fn pump(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), EncoderError> {
        let Some(events) = self.events.clone() else {
            return Ok(());
        };
        loop {
            // SAFETY: consulta não bloqueante da fila de eventos do MFT.
            let event = match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(e) => e,
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(()),
                Err(e) => return Err(EncoderError::win("GetEvent", e)),
            };
            // SAFETY: evento válido.
            let kind = unsafe { event.GetType() }.map_err(win("GetType"))?;
            if kind == METransformNeedInput.0 as u32 {
                self.need_input += 1;
            } else if kind == METransformHaveOutput.0 as u32 {
                self.process_output(out)?;
            }
        }
    }

    fn drain_sync(&mut self, out: &mut Vec<EncodedFrame>) -> Result<(), EncoderError> {
        while self.process_output(out)? {}
        Ok(())
    }

    /// Lê uma saída. Retorna `false` quando o encoder precisa de mais entrada.
    fn process_output(&mut self, out: &mut Vec<EncodedFrame>) -> Result<bool, EncoderError> {
        let own_sample = if self.provides_samples {
            None
        } else {
            // SAFETY: criação de amostra e buffer em memória do sistema.
            unsafe {
                let sample = MFCreateSample().map_err(win("MFCreateSample"))?;
                let buffer = MFCreateMemoryBuffer(self.output_buffer_size)
                    .map_err(win("MFCreateMemoryBuffer"))?;
                sample.AddBuffer(&buffer).map_err(win("AddBuffer"))?;
                Some(sample)
            }
        };
        let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: std::mem::ManuallyDrop::new(own_sample),
            dwStatus: 0,
            pEvents: std::mem::ManuallyDrop::new(None),
        }];
        let mut status = 0u32;
        // SAFETY: array de saída com um elemento válido; os campos ManuallyDrop são
        // recuperados logo abaixo para não vazar referências.
        let result = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
        // SAFETY: recupera a posse dos ponteiros colocados ou preenchidos pelo MFT.
        let (sample, _events) = unsafe {
            (
                std::mem::ManuallyDrop::take(&mut buffers[0].pSample),
                std::mem::ManuallyDrop::take(&mut buffers[0].pEvents),
            )
        };
        match result {
            Ok(()) => {}
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(false),
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                self.renegotiate_output()?;
                return Ok(true);
            }
            Err(e) => return Err(EncoderError::win("ProcessOutput", e)),
        }
        if let Some(sample) = sample {
            if let Some(frame) = self.read_sample(&sample)? {
                out.push(frame);
            }
        }
        Ok(true)
    }

    fn renegotiate_output(&mut self) -> Result<(), EncoderError> {
        // SAFETY: consulta e definição do tipo de saída após mudança de fluxo.
        unsafe {
            let available = self
                .transform
                .GetOutputAvailableType(0, 0)
                .map_err(win("GetOutputAvailableType"))?;
            self.transform
                .SetOutputType(0, &available, 0)
                .map_err(win("SetOutputType"))?;
            if let Ok(current) = self.transform.GetOutputCurrentType(0) {
                read_sequence_header(&current, &mut self.params);
            }
        }
        Ok(())
    }

    fn read_sample(&mut self, sample: &IMFSample) -> Result<Option<EncodedFrame>, EncoderError> {
        // SAFETY: leitura de uma amostra de saída válida; o buffer é desbloqueado antes de sair.
        let (data, pts, clean) = unsafe {
            let pts = sample.GetSampleTime().unwrap_or(0);
            // Só IDR conta como keyframe (WebCodecs exige IDR em chunks "key");
            // CleanPoint sem IDR é apenas registrado.
            let clean = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) == 1;
            let buffer = sample
                .ConvertToContiguousBuffer()
                .map_err(win("ConvertToContiguousBuffer"))?;
            let mut ptr = std::ptr::null_mut();
            let mut len = 0u32;
            buffer
                .Lock(&mut ptr, None, Some(&mut len))
                .map_err(win("Lock"))?;
            let data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
            let _ = buffer.Unlock();
            (data, pts, clean)
        };
        let au = annexb::normalize(&data, &mut self.params);
        if clean && au.as_ref().is_some_and(|a| !a.keyframe) {
            tracing::debug!("CleanPoint sem IDR");
        }
        Ok(au.map(|au| EncodedFrame {
            data: au.data,
            pts_hns: pts,
            keyframe: au.keyframe,
        }))
    }

    fn make_sample(&self, nv12: &ID3D11Texture2D, pts_hns: i64) -> Result<IMFSample, EncoderError> {
        // SAFETY: criação de amostra; a textura NV12 permanece viva no rodízio do conversor.
        unsafe {
            let sample = MFCreateSample().map_err(win("MFCreateSample"))?;
            match &self.input {
                InputPath::Gpu { .. } => {
                    let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, nv12, 0, false)
                        .map_err(win("MFCreateDXGISurfaceBuffer"))?;
                    if let Ok(two_d) = buffer.cast::<IMF2DBuffer>() {
                        if let Ok(len) = two_d.GetContiguousLength() {
                            let _ = buffer.SetCurrentLength(len);
                        }
                    }
                    sample.AddBuffer(&buffer).map_err(win("AddBuffer"))?;
                }
                InputPath::Cpu { staging, context } => {
                    let buffer = copy_nv12_to_memory(
                        context,
                        staging,
                        nv12,
                        self.config.width,
                        self.config.height,
                    )?;
                    sample.AddBuffer(&buffer).map_err(win("AddBuffer"))?;
                }
            }
            sample
                .SetSampleTime(pts_hns)
                .map_err(win("SetSampleTime"))?;
            sample
                .SetSampleDuration(self.frame_duration)
                .map_err(win("SetSampleDuration"))?;
            Ok(sample)
        }
    }
}

impl Drop for H264Encoder {
    fn drop(&mut self) {
        // SAFETY: mensagens de encerramento; erros são irrelevantes no descarte.
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}

fn h264_type(config: &EncoderConfig, high: bool) -> Result<IMFMediaType, EncoderError> {
    let profile = if high {
        eAVEncH264VProfile_High.0
    } else {
        eAVEncH264VProfile_Main.0
    };
    // SAFETY: criação e preenchimento de um tipo de mídia próprio.
    unsafe {
        let t = MFCreateMediaType().map_err(win("MFCreateMediaType"))?;
        t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(win("MF_MT_MAJOR_TYPE"))?;
        t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
            .map_err(win("MF_MT_SUBTYPE"))?;
        t.SetUINT32(&MF_MT_AVG_BITRATE, bitrate_bps(config.bitrate_kbps))
            .map_err(win("MF_MT_AVG_BITRATE"))?;
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(config.width, config.height))
            .map_err(win("MF_MT_FRAME_SIZE"))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(config.fps, 1))
            .map_err(win("MF_MT_FRAME_RATE"))?;
        t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))
            .map_err(win("MF_MT_PIXEL_ASPECT_RATIO"))?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(win("MF_MT_INTERLACE_MODE"))?;
        t.SetUINT32(&MF_MT_MPEG2_PROFILE, profile as u32)
            .map_err(win("MF_MT_MPEG2_PROFILE"))?;
        Ok(t)
    }
}

/// Tipo de entrada NV12: usa o tipo oferecido pelo MFT quando existir (alguns
/// encoders de hardware exigem atributos próprios) e completa tamanho e taxa.
fn nv12_type(
    transform: &IMFTransform,
    config: &EncoderConfig,
) -> Result<IMFMediaType, EncoderError> {
    // SAFETY: enumeração de tipos de entrada oferecidos pelo MFT; tipos criados por nós.
    unsafe {
        let mut chosen = None;
        for i in 0..32 {
            let Ok(t) = transform.GetInputAvailableType(0, i) else {
                break;
            };
            if t.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) {
                chosen = Some(t);
                break;
            }
        }
        let t = match chosen {
            Some(t) => t,
            None => {
                let t = MFCreateMediaType().map_err(win("MFCreateMediaType"))?;
                t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
                    .map_err(win("MF_MT_MAJOR_TYPE"))?;
                t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
                    .map_err(win("MF_MT_SUBTYPE"))?;
                t
            }
        };
        t.SetUINT64(&MF_MT_FRAME_SIZE, pack(config.width, config.height))
            .map_err(win("MF_MT_FRAME_SIZE"))?;
        t.SetUINT64(&MF_MT_FRAME_RATE, pack(config.fps, 1))
            .map_err(win("MF_MT_FRAME_RATE"))?;
        t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(win("MF_MT_INTERLACE_MODE"))?;
        Ok(t)
    }
}

fn read_sequence_header(media_type: &IMFMediaType, params: &mut ParameterSets) {
    let mut ptr = std::ptr::null_mut();
    let mut len = 0u32;
    // SAFETY: blob alocado pelo MF e liberado com CoTaskMemFree.
    unsafe {
        if media_type
            .GetAllocatedBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut ptr, &mut len)
            .is_ok()
            && !ptr.is_null()
        {
            let blob = std::slice::from_raw_parts(ptr, len as usize);
            params.update_from(blob);
            CoTaskMemFree(Some(ptr as *const _));
        }
    }
}

fn immediate_context(device: &ID3D11Device) -> Result<ID3D11DeviceContext, EncoderError> {
    // SAFETY: dispositivo válido.
    unsafe { device.GetImmediateContext() }.map_err(win("GetImmediateContext"))
}

fn create_staging(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D, EncoderError> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
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
    // SAFETY: descritor válido; saída local.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture)) }
        .map_err(win("CreateTexture2D staging"))?;
    texture.ok_or(EncoderError::Platform {
        context: "CreateTexture2D staging",
        message: "nula".into(),
    })
}

/// Copia uma textura NV12 para um buffer de memória contíguo (somente no fallback de software).
fn copy_nv12_to_memory(
    context: &ID3D11DeviceContext,
    staging: &ID3D11Texture2D,
    nv12: &ID3D11Texture2D,
    width: u32,
    height: u32,
) -> Result<windows::Win32::Media::MediaFoundation::IMFMediaBuffer, EncoderError> {
    let (w, h) = (width as usize, height as usize);
    let size = w * h * 3 / 2;
    // SAFETY: cópia GPU→staging, mapeamento de leitura e cópia linha a linha
    // respeitando o pitch; o mapeamento e o buffer são liberados antes de sair.
    unsafe {
        context.CopyResource(staging, nv12);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context
            .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(win("Map"))?;
        let buffer = MFCreateMemoryBuffer(size as u32).map_err(win("MFCreateMemoryBuffer"))?;
        let mut dst = std::ptr::null_mut();
        if let Err(e) = buffer.Lock(&mut dst, None, None) {
            context.Unmap(staging, 0);
            return Err(EncoderError::win("Lock", e));
        }
        let pitch = mapped.RowPitch as usize;
        let src = mapped.pData as *const u8;
        // Plano Y (h linhas) seguido do plano UV intercalado (h/2 linhas).
        for row in 0..(h + h / 2) {
            std::ptr::copy_nonoverlapping(src.add(row * pitch), dst.add(row * w), w);
        }
        let _ = buffer.Unlock();
        context.Unmap(staging, 0);
        buffer
            .SetCurrentLength(size as u32)
            .map_err(win("SetCurrentLength"))?;
        Ok(buffer)
    }
}
