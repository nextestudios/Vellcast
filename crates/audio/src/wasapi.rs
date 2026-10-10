//! Captura WASAPI no Windows.
//!
//! Uma thread dedicada (COM MTA) abre o cliente de áudio, espera pelo evento
//! do WASAPI (sem polling ativo), converte, monta blocos de 20 ms e codifica
//! em Opus. Falhas do dispositivo reabrem a captura com backoff.
//!
//! Sem loopback (áudio do jogo desligado), a mesma thread completa a linha do
//! tempo com silêncio a cada 20 ms. O microfone, quando ligado, roda numa
//! thread própria (`vellcast-mic`) que só enche a [`MicFifo`]; a thread
//! principal mistura o microfone em cada bloco antes do Opus.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    ActivateAudioInterfaceAsync, DEVICE_STATE_ACTIVE, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    PROCESS_LOOPBACK_MODE, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVE_FORMAT_PCM, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eCapture, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{
    PROPVARIANT, PropVariantClear, PropVariantToStringAlloc,
};
use windows::Win32::System::Com::{
    BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemAlloc,
    CoTaskMemFree, CoUninitialize, STGM_READ,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{
    CreateEventW, GetCurrentProcessId, SetEvent, WaitForSingleObject,
};
use windows::Win32::System::Variant::VT_BLOB;
use windows_core::{HRESULT, IUnknown, Interface, implement};

use crate::convert::{Converter, InputFormat, SampleFormat};
use crate::frames::FrameAssembler;
use crate::mix::{MicFifo, mix_into};
use crate::opus_encoder::OpusEncoder;
use crate::process::{ProcessEntry, choose_exclusion};
use crate::{
    AudioError, AudioPacket, AudioSetup, AudioTarget, CHANNELS, FRAME_SAMPLES, InputDevice,
    MicConfig, MicState, PacketSink, SAMPLE_RATE, Shared, WarningSink,
};

/// Espera máxima pelo evento do WASAPI; também define a frequência de checagem de parada.
const WAIT_MS: u32 = 20;
/// Duração do buffer compartilhado pedida ao WASAPI (100 ms, em unidades de 100 ns).
const BUFFER_HNS: i64 = 1_000_000;
/// Durante silêncio sem pacotes, a linha do tempo é completada até `agora - LAG`.
/// A folga evita cortar dados reais que chegam com a latência do dispositivo.
const SILENCE_FILL_LAG_US: u64 = 100_000;
/// Sem loopback não há latência de dispositivo a esperar: folga de dois blocos.
const SILENCE_ONLY_LAG_US: u64 = 40_000;
/// Tentativas de reabrir o microfone antes de desistir (dispositivo removido).
const MIC_MAX_RETRIES: usize = 4;
/// Intervalo de checagem do dispositivo padrão no loopback clássico.
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// Tempo máximo para abrir o dispositivo antes de `start` retornar.
const START_TIMEOUT: Duration = Duration::from_secs(5);
const RESTART_BACKOFF: [Duration; 4] = [
    Duration::from_millis(200),
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
];

fn wasapi_err(e: windows_core::Error) -> AudioError {
    AudioError::Wasapi(format!("{} ({:#010x})", e.message(), e.code().0))
}

pub(crate) fn spawn(
    setup: AudioSetup,
    origin_qpc: i64,
    on_packet: PacketSink,
    on_warning: WarningSink,
    shared: Arc<Shared>,
) -> Result<JoinHandle<()>, AudioError> {
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<(), AudioError>>(1);
    let thread_shared = Arc::clone(&shared);
    let handle = std::thread::Builder::new()
        .name("vellcast-audio".into())
        .spawn(move || {
            let on_warning: Arc<WarningSink> = Arc::new(on_warning);
            let mut capture = CaptureThread {
                initial: setup,
                clock: QpcClock::new(origin_qpc),
                on_warning: Arc::clone(&on_warning),
                shared: Arc::clone(&thread_shared),
                mic: None,
                retired_mics: Vec::new(),
            };
            capture.run(on_packet, ready_tx);
        })
        .map_err(|e| AudioError::Wasapi(e.to_string()))?;

    match ready_rx.recv_timeout(START_TIMEOUT) {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => {
            let _ = handle.join();
            Err(e)
        }
        Err(_) => {
            // A thread encerra sozinha ao ver `stop`.
            shared.stop.store(true, Ordering::Relaxed);
            Err(AudioError::Timeout)
        }
    }
}

/// Valor bruto atual do `QueryPerformanceCounter`.
pub(crate) fn qpc_ticks() -> i64 {
    let mut ticks = 0i64;
    // SAFETY: ponteiro para variável local válida; a função apenas a escreve.
    let _ = unsafe { QueryPerformanceCounter(&mut ticks) };
    ticks
}

/// Conversão entre QPC e o PTS da transmissão.
struct QpcClock {
    /// Origem em unidades de 100 ns.
    origin_hns: u64,
    frequency: i64,
}

impl QpcClock {
    fn new(origin_qpc: i64) -> Self {
        let mut frequency = 0i64;
        // SAFETY: ponteiro para variável local válida; a função apenas a escreve.
        let _ = unsafe { QueryPerformanceFrequency(&mut frequency) };
        let frequency = frequency.max(1);
        Self {
            origin_hns: Self::ticks_to_hns(origin_qpc, frequency),
            frequency,
        }
    }

    fn ticks_to_hns(ticks: i64, frequency: i64) -> u64 {
        (i128::from(ticks.max(0)) * 10_000_000 / i128::from(frequency)) as u64
    }

    /// PTS (µs) de uma posição QPC em unidades de 100 ns (como o `GetBuffer` devolve).
    fn pts_from_hns(&self, hns: u64) -> u64 {
        hns.saturating_sub(self.origin_hns) / 10
    }

    fn now_pts(&self) -> u64 {
        self.pts_from_hns(Self::ticks_to_hns(qpc_ticks(), self.frequency))
    }
}

/// Inicializa COM (MTA) na thread e o finaliza ao sair.
struct ComGuard {
    owned: bool,
}

impl ComGuard {
    fn init() -> Self {
        // SAFETY: chamada na própria thread de captura, antes de qualquer objeto COM.
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        // RPC_E_CHANGED_MODE: COM já iniciado em outro modo; seguimos sem finalizar.
        Self { owned: hr.is_ok() }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: pareado com o `CoInitializeEx` bem-sucedido desta thread.
            unsafe { CoUninitialize() };
        }
    }
}

/// Evento Win32 de reset automático, fechado no `Drop`.
struct Event(HANDLE);

impl Event {
    fn new() -> Result<Self, AudioError> {
        // SAFETY: parâmetros opcionais nulos; o handle retornado passa a ser deste objeto.
        let handle = unsafe { CreateEventW(None, false, false, None) }.map_err(wasapi_err)?;
        Ok(Self(handle))
    }

    /// Espera o evento; retorna `true` se foi sinalizado.
    fn wait(&self, ms: u32) -> bool {
        // SAFETY: handle válido enquanto `self` existe.
        let r = unsafe { WaitForSingleObject(self.0, ms) };
        r == WAIT_OBJECT_0
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: handle criado por `CreateEventW` e fechado uma única vez.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Handler da ativação assíncrona; só sinaliza um evento.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion {
    /// Valor bruto do `HANDLE` do evento (o tipo `HANDLE` não é `Send`).
    event: usize,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _operation: windows_core::Ref<IActivateAudioInterfaceAsyncOperation>,
    ) -> windows_core::Result<()> {
        // SAFETY: o evento pertence a `activate_process_loopback`, que espera por
        // este sinal antes de destruí-lo.
        unsafe { SetEvent(HANDLE(self.event as *mut core::ffi::c_void)) }
    }
}

/// Origem efetiva do áudio.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    ProcessLoopback {
        pid: u32,
        mode: PROCESS_LOOPBACK_MODE,
    },
    /// Loopback clássico do dispositivo de saída padrão.
    Endpoint,
    /// Dispositivo de entrada (microfone); `None` = padrão do Windows.
    Microphone(Option<String>),
}

/// Formato pedido ao WASAPI: 48 kHz, estéreo, `f32`.
fn desired_format() -> WAVEFORMATEX {
    let block_align = u16::from(CHANNELS) * 4;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: u16::from(CHANNELS),
        nSamplesPerSec: SAMPLE_RATE,
        nAvgBytesPerSec: SAMPLE_RATE * u32::from(block_align),
        nBlockAlign: block_align,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

fn native_input() -> InputFormat {
    InputFormat {
        format: SampleFormat::F32,
        channels: u16::from(CHANNELS),
        sample_rate: SAMPLE_RATE,
        channel_mask: 0,
    }
}

/// Lê um `WAVEFORMATEX` (possivelmente `EXTENSIBLE`) devolvido pelo WASAPI.
///
/// # Safety
/// `ptr` deve apontar para um `WAVEFORMATEX` válido, com `cbSize` bytes extras.
unsafe fn parse_format(ptr: *const WAVEFORMATEX) -> Option<InputFormat> {
    // SAFETY: garantido pelo chamador; leitura não alinhada porque a struct é `packed`.
    let base = unsafe { std::ptr::read_unaligned(ptr) };
    let tag = u32::from(base.wFormatTag);
    let bits = base.wBitsPerSample;
    let (float, valid_bits, mask) = if tag == WAVE_FORMAT_EXTENSIBLE && base.cbSize >= 22 {
        // SAFETY: `cbSize >= 22` garante que a extensão inteira está presente.
        let ext = unsafe { std::ptr::read_unaligned(ptr.cast::<WAVEFORMATEXTENSIBLE>()) };
        let sub = ext.SubFormat;
        // SAFETY: todos os campos da união são `u16`; qualquer valor é válido.
        let valid = unsafe { ext.Samples.wValidBitsPerSample };
        if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            (true, valid, ext.dwChannelMask)
        } else if sub == KSDATAFORMAT_SUBTYPE_PCM {
            (false, valid, ext.dwChannelMask)
        } else {
            return None;
        }
    } else if tag == WAVE_FORMAT_IEEE_FLOAT {
        (true, bits, 0)
    } else if tag == WAVE_FORMAT_PCM {
        (false, bits, 0)
    } else {
        return None;
    };
    let format = match (float, bits, valid_bits) {
        (true, 32, _) => SampleFormat::F32,
        (false, 16, _) => SampleFormat::I16,
        (false, 24, _) => SampleFormat::I24,
        (false, 32, _) => SampleFormat::I32,
        _ => return None,
    };
    Some(InputFormat {
        format,
        channels: base.nChannels,
        sample_rate: base.nSamplesPerSec,
        channel_mask: mask,
    })
}

/// Lista os processos em execução (para localizar o Discord).
fn list_processes() -> Vec<ProcessEntry> {
    let mut out = Vec::new();
    // SAFETY: snapshot de processos; o handle é fechado abaixo.
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return out;
    };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry` tem `dwSize` correto e vive durante toda a iteração.
    let mut ok = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
    while ok {
        let name = entry.szExeFile;
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        out.push(ProcessEntry {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
            exe_name: String::from_utf16_lossy(&name[..len]),
        });
        // SAFETY: mesmo snapshot e `entry` válidos.
        ok = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
    }
    // SAFETY: handle devolvido por `CreateToolhelp32Snapshot`, fechado uma vez.
    let _ = unsafe { CloseHandle(snapshot) };
    out
}

/// Ativa o dispositivo virtual de loopback por processo (Windows 10 2004+).
fn activate_process_loopback(
    pid: u32,
    mode: PROCESS_LOOPBACK_MODE,
) -> Result<IAudioClient, AudioError> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: mode,
            },
        },
    };
    // O blob dos parâmetros é alocado com `CoTaskMemAlloc` e nunca liberado por
    // nós: o diagnóstico com page heap (workflow diag-audio) mostrou que a
    // operação de ativação libera esse bloco ao terminar. Com memória da pilha ou
    // de `Box`, isso corrompia o heap (0xC0000374) ou causava liberação dupla.
    let size = std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>();
    // SAFETY: alocação simples; o resultado é verificado antes do uso.
    let blob = unsafe { CoTaskMemAlloc(size) }.cast::<AUDIOCLIENT_ACTIVATION_PARAMS>();
    if blob.is_null() {
        return Err(AudioError::Wasapi("sem memória para a ativação".into()));
    }
    // SAFETY: `blob` aponta para `size` bytes recém-alocados, alinhados para a struct.
    unsafe { blob.write(params) };
    // O PROPVARIANT fica no heap para continuar válido mesmo se a ativação
    // passar do prazo (nesse caso ele é vazado).
    let mut prop = Box::new(PROPVARIANT::default());
    // SAFETY: monta um PROPVARIANT VT_BLOB que aponta para `blob`.
    unsafe {
        let inner = &mut *prop.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB {
            cbSize: size as u32,
            pBlobData: blob.cast::<u8>(),
        };
    }

    tracing::debug!(pid, mode = mode.0, "áudio: ativando loopback por processo");
    let done = Event::new()?;
    let handler: IActivateAudioInterfaceCompletionHandler = Completion {
        event: done.0.0 as usize,
    }
    .into();
    // SAFETY: todos os ponteiros (IID, PROPVARIANT, handler) são válidos durante a chamada
    // e, no caso do PROPVARIANT, até a conclusão sinalizada em `done`.
    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(std::ptr::from_ref::<PROPVARIANT>(&prop)),
            &handler,
        )
    }
    .map_err(|e| AudioError::ProcessLoopbackUnavailable(e.message().to_string()))?;

    tracing::debug!("áudio: ativação pedida; aguardando conclusão");
    if !done.wait(START_TIMEOUT.as_millis() as u32) {
        // A operação pode concluir depois: mantém o PROPVARIANT, o evento e o handler
        // vivos para sempre em vez de liberar memória que o Windows ainda vai ler.
        std::mem::forget(done);
        std::mem::forget(handler);
        std::mem::forget(operation);
        Box::leak(prop);
        return Err(AudioError::Timeout);
    }
    tracing::debug!("áudio: ativação concluída");
    let mut result = HRESULT(0);
    let mut unknown: Option<IUnknown> = None;
    // SAFETY: a operação terminou (evento sinalizado); os ponteiros de saída são locais.
    unsafe { operation.GetActivateResult(&mut result, &mut unknown) }
        .map_err(|e| AudioError::ProcessLoopbackUnavailable(e.message().to_string()))?;
    result
        .ok()
        .map_err(|e| AudioError::ProcessLoopbackUnavailable(e.message().to_string()))?;
    unknown
        .ok_or_else(|| AudioError::ProcessLoopbackUnavailable("ativação sem interface".into()))?
        .cast::<IAudioClient>()
        .map_err(wasapi_err)
}

fn device_enumerator() -> Result<IMMDeviceEnumerator, AudioError> {
    // SAFETY: COM inicializado nesta thread por `ComGuard`.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(wasapi_err)
}

/// Abre um dispositivo de entrada pelo ID (ou o padrão).
fn open_input_device(
    enumerator: &IMMDeviceEnumerator,
    id: Option<&str>,
) -> Result<IMMDevice, AudioError> {
    match id {
        Some(id) => {
            let wide: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
            // SAFETY: `wide` é uma string UTF-16 terminada em zero que vive durante a chamada.
            unsafe { enumerator.GetDevice(windows_core::PCWSTR(wide.as_ptr())) }
                .map_err(|e| AudioError::NoDevice(e.message().to_string()))
        }
        // SAFETY: enumerador válido.
        None => unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) }
            .map_err(|e| AudioError::NoDevice(e.message().to_string())),
    }
}

/// Um cliente de captura aberto e iniciado.
struct Stream {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    event: Event,
    /// `None` quando o WASAPI já entrega 48 kHz estéreo `f32`.
    converter: Option<Converter>,
    frame_bytes: usize,
    /// ID do dispositivo padrão no momento da abertura (loopback clássico).
    device_id: Option<String>,
}

impl Stream {
    fn open(source: &Source) -> Result<Self, AudioError> {
        let flags = match source {
            Source::Microphone(_) => AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            _ => AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
        };
        let convert = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        let desired = desired_format();

        let (client, device_id) = match source {
            Source::ProcessLoopback { pid, mode } => {
                (activate_process_loopback(*pid, *mode)?, None)
            }
            Source::Endpoint => {
                let enumerator = device_enumerator()?;
                // SAFETY: enumerador válido.
                let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
                    .map_err(|e| AudioError::NoDevice(e.message().to_string()))?;
                let id = default_device_id(&enumerator);
                // SAFETY: dispositivo válido; sem parâmetros de ativação.
                let client: IAudioClient =
                    unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(wasapi_err)?;
                (client, id)
            }
            Source::Microphone(id) => {
                let enumerator = device_enumerator()?;
                let device = open_input_device(&enumerator, id.as_deref())?;
                // SAFETY: dispositivo válido; sem parâmetros de ativação.
                let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
                    .map_err(|e| AudioError::NoDevice(e.message().to_string()))?;
                (client, device_id(&device))
            }
        };

        tracing::debug!(?source, "áudio: inicializando o cliente");
        // Primeiro tenta deixar o Windows converter para 48 kHz estéreo f32.
        // SAFETY: `desired` vive durante a chamada; cliente ainda não inicializado.
        let auto = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags | convert,
                BUFFER_HNS,
                0,
                &desired,
                None,
            )
        };
        let (input, converter) = match auto {
            Ok(()) => (native_input(), None),
            Err(first) if !matches!(source, Source::ProcessLoopback { .. }) => {
                // Sem conversão automática: usa o formato de mixagem e converte aqui
                // (downmix e reamostragem para 48 kHz estéreo).
                // SAFETY: o WASAPI aloca o formato; liberado com `CoTaskMemFree` abaixo.
                let mix = unsafe { client.GetMixFormat() }.map_err(wasapi_err)?;
                // SAFETY: `mix` aponta para um WAVEFORMATEX válido devolvido pelo WASAPI.
                let parsed = unsafe { parse_format(mix) };
                // SAFETY: `mix` vive durante a chamada.
                let init = unsafe {
                    client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, BUFFER_HNS, 0, mix, None)
                };
                // SAFETY: memória alocada pelo WASAPI com `CoTaskMemAlloc`, liberada uma vez.
                unsafe { CoTaskMemFree(Some(mix.cast_const().cast())) };
                init.map_err(|e| {
                    AudioError::Wasapi(format!(
                        "{}; formato nativo: {}",
                        first.message(),
                        e.message()
                    ))
                })?;
                let input = parsed
                    .ok_or_else(|| AudioError::Wasapi("formato de mixagem não suportado".into()))?;
                (input, Some(Converter::new(input)))
            }
            Err(first) => {
                // Loopback por processo: tenta sem as flags de conversão.
                // SAFETY: `desired` vive durante a chamada.
                unsafe {
                    client.Initialize(
                        AUDCLNT_SHAREMODE_SHARED,
                        flags,
                        BUFFER_HNS,
                        0,
                        &desired,
                        None,
                    )
                }
                .map_err(|e| {
                    AudioError::ProcessLoopbackUnavailable(format!(
                        "{}; {}",
                        first.message(),
                        e.message()
                    ))
                })?;
                (native_input(), None)
            }
        };

        tracing::debug!("áudio: cliente inicializado; configurando evento e serviço de captura");
        let event = Event::new()?;
        // SAFETY: cliente inicializado com EVENTCALLBACK; o evento vive tanto quanto o `Stream`
        // (campo declarado depois de `client`, destruído depois do `Stop` em `Drop`).
        unsafe { client.SetEventHandle(event.0) }.map_err(wasapi_err)?;
        // SAFETY: cliente inicializado.
        let capture: IAudioCaptureClient = unsafe { client.GetService() }.map_err(wasapi_err)?;
        // SAFETY: cliente inicializado e com evento configurado.
        unsafe { client.Start() }.map_err(wasapi_err)?;
        Ok(Self {
            client,
            capture,
            event,
            converter,
            frame_bytes: input.frame_bytes(),
            device_id,
        })
    }

    /// Lê todos os pacotes disponíveis. `on_data` recebe as amostras (48 kHz
    /// estéreo, já com `gain`) e o PTS QPC em unidades de 100 ns (0 se ausente).
    /// Com `discard`, os pacotes são liberados sem conversão (microfone mudo).
    fn drain(
        &mut self,
        samples: &mut Vec<f32>,
        gain: f32,
        discard: bool,
        on_data: &mut dyn FnMut(&[f32], u64),
    ) -> Result<(), windows_core::Error> {
        loop {
            // SAFETY: cliente de captura válido.
            let next = unsafe { self.capture.GetNextPacketSize() }?;
            if next == 0 {
                return Ok(());
            }
            let mut data: *mut u8 = std::ptr::null_mut();
            let mut frames = 0u32;
            let mut flags = 0u32;
            let mut qpc_hns = 0u64;
            // SAFETY: ponteiros de saída locais; o buffer é válido até `ReleaseBuffer`.
            unsafe {
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc_hns))
            }?;
            if !discard {
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null();
                let len = frames as usize * self.frame_bytes;
                let bytes: &[u8] = if silent {
                    &[]
                } else {
                    // SAFETY: o WASAPI garante `frames * nBlockAlign` bytes válidos em `data`
                    // até `ReleaseBuffer`, chamado logo após o uso.
                    unsafe { std::slice::from_raw_parts(data, len) }
                };
                samples.clear();
                match self.converter.as_mut() {
                    Some(conv) => conv.convert(bytes, frames as usize, silent, gain, samples),
                    None if silent => samples.resize(frames as usize * 2, 0.0),
                    None => samples.extend(bytes.as_chunks::<4>().0.iter().map(|b| {
                        (f32::from_le_bytes([b[0], b[1], b[2], b[3]]) * gain).clamp(-1.0, 1.0)
                    })),
                }
            }
            // SAFETY: libera exatamente o que foi obtido em `GetBuffer`.
            unsafe { self.capture.ReleaseBuffer(frames) }?;
            if !discard {
                on_data(samples, qpc_hns);
            }
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: cliente válido; parar antes de liberar o evento associado.
        let _ = unsafe { self.client.Stop() };
    }
}

/// ID de um dispositivo (string alocada pelo COM, copiada e liberada).
fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: dispositivo válido; a string é alocada pelo COM e liberada abaixo.
    let id = unsafe { device.GetId() }.ok()?;
    // SAFETY: `id` é uma string UTF-16 terminada em zero válida.
    let text = unsafe { id.to_string() }.ok();
    // SAFETY: memória alocada pelo COM, liberada uma vez.
    unsafe { CoTaskMemFree(Some(id.0.cast_const().cast())) };
    text
}

/// Nome amigável (ex.: "Microfone (Headset USB)").
fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: dispositivo válido; acesso só de leitura.
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }.ok()?;
    // SAFETY: chave constante válida; o PROPVARIANT devolvido é liberado abaixo.
    let mut value = unsafe { store.GetValue(&PKEY_Device_FriendlyName) }.ok()?;
    // SAFETY: `value` é um PROPVARIANT válido; a string devolvida é alocada pelo COM.
    let text = unsafe { PropVariantToStringAlloc(&value) }
        .ok()
        .and_then(|p| {
            // SAFETY: `p` é uma string UTF-16 terminada em zero devolvida pelo COM.
            let s = unsafe { p.to_string() }.ok();
            // SAFETY: memória alocada pelo COM, liberada uma vez.
            unsafe { CoTaskMemFree(Some(p.0.cast_const().cast())) };
            s
        });
    // SAFETY: libera o conteúdo do PROPVARIANT uma única vez.
    let _ = unsafe { PropVariantClear(&mut value) };
    text.filter(|t| !t.trim().is_empty())
}

fn default_device_id(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    // SAFETY: enumerador válido.
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
    device_id(&device)
}

fn current_default_device_id() -> Option<String> {
    default_device_id(&device_enumerator().ok()?)
}

/// Microfones ativos, com o padrão do Windows marcado. Sem nenhum, lista vazia.
pub(crate) fn list_input_devices() -> Result<Vec<InputDevice>, AudioError> {
    let _com = ComGuard::init();
    let enumerator = device_enumerator()?;
    // SAFETY: enumerador válido.
    let default = unsafe { enumerator.GetDefaultAudioEndpoint(eCapture, eConsole) }
        .ok()
        .and_then(|d| device_id(&d));
    // SAFETY: enumerador válido; só dispositivos ativos (conectados e habilitados).
    let collection = unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }
        .map_err(wasapi_err)?;
    // SAFETY: coleção válida.
    let count = unsafe { collection.GetCount() }.map_err(wasapi_err)?;
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        // SAFETY: índice dentro de `count`.
        let Ok(device) = (unsafe { collection.Item(i) }) else {
            continue;
        };
        let Some(id) = device_id(&device) else {
            continue;
        };
        let name = friendly_name(&device).unwrap_or_else(|| id.clone());
        out.push(InputDevice {
            is_default: default.as_deref() == Some(id.as_str()),
            id,
            name,
        });
    }
    Ok(out)
}

/// Motivo de saída do laço de uma origem.
enum Exit {
    Stopped,
    /// O dono pediu outra origem de loopback (`None` = só silêncio).
    Reconfigure(Option<AudioTarget>),
    Restart(String),
}

/// Codifica os blocos de 20 ms, misturando o microfone quando ativo.
struct Emitter {
    encoder: OpusEncoder,
    on_packet: PacketSink,
    on_warning: Arc<WarningSink>,
    shared: Arc<Shared>,
    mixed: Vec<f32>,
    mic: Vec<f32>,
    encode_failed: bool,
}

impl Emitter {
    fn emit(&mut self, frame: &[f32], pts_us: u64) {
        let Self {
            encoder,
            on_packet,
            on_warning,
            shared,
            mixed,
            mic,
            encode_failed,
        } = self;
        let mut data = frame;
        if shared.mic_state() == MicState::Active
            && !shared.mic_muted.load(Ordering::Relaxed)
            && shared.mic_fifo().pop_into(mic)
        {
            mixed.clear();
            mixed.extend_from_slice(frame);
            mix_into(mixed, mic, 1.0);
            data = mixed;
        }
        match encoder.encode(data) {
            Ok(packet) => {
                shared.packets.fetch_add(1, Ordering::Relaxed);
                on_packet(AudioPacket {
                    data: packet,
                    pts_us,
                });
            }
            Err(e) => {
                if !*encode_failed {
                    *encode_failed = true;
                    on_warning(format!("Falha ao codificar áudio: {e}"));
                }
            }
        }
    }
}

/// Uma thread de microfone em execução.
struct MicHandle {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct CaptureThread {
    initial: AudioSetup,
    clock: QpcClock,
    on_warning: Arc<WarningSink>,
    shared: Arc<Shared>,
    mic: Option<MicHandle>,
    /// Threads de microfone antigas, já avisadas para parar.
    retired_mics: Vec<MicHandle>,
}

impl CaptureThread {
    fn stopping(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }

    fn warn(&self, message: String) {
        (self.on_warning)(message);
    }

    /// Decide a origem e abre o `Stream`, com fallback no modo sistema.
    fn open_target(&self, target: AudioTarget) -> Result<(Source, Stream), AudioError> {
        match target {
            AudioTarget::Process(pid) => {
                let source = Source::ProcessLoopback {
                    pid,
                    mode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                };
                let stream = Stream::open(&source)?;
                Ok((source, stream))
            }
            AudioTarget::System => {
                // SAFETY: sem pré-condições.
                let own = unsafe { GetCurrentProcessId() };
                let processes = list_processes();
                tracing::debug!(count = processes.len(), "áudio: processos listados");
                let exclusion = choose_exclusion(&processes, own);
                tracing::debug!(pid = exclusion.pid, "áudio: árvore de processos excluída");
                if let Some(w) = &exclusion.warning {
                    self.warn(w.clone());
                }
                let source = Source::ProcessLoopback {
                    pid: exclusion.pid,
                    mode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
                };
                match Stream::open(&source) {
                    Ok(stream) => Ok((source, stream)),
                    Err(e) => {
                        tracing::warn!(error = %e, "loopback por processo indisponível; usando o dispositivo padrão");
                        self.warn(
                            "Capturando todo o áudio do sistema: o áudio do Discord (voz) pode ser transmitido."
                                .into(),
                        );
                        let stream = Stream::open(&Source::Endpoint)?;
                        Ok((Source::Endpoint, stream))
                    }
                }
            }
        }
    }

    /// Troca de origem pedida durante a transmissão: nunca falha; no pior caso,
    /// fica só com silêncio (e o microfone) e avisa.
    fn open_live(&self, target: AudioTarget) -> Option<(Source, Stream)> {
        match self.open_target(target) {
            Ok(v) => Some(v),
            Err(AudioError::ProcessLoopbackUnavailable(detail))
                if matches!(target, AudioTarget::Process(_)) =>
            {
                self.warn(format!(
                    "Áudio só do aplicativo indisponível ({detail}). Transmitindo o áudio do sistema, sem o Discord."
                ));
                match self.open_target(AudioTarget::System) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        self.warn(format!("Áudio do jogo desativado: {e}"));
                        None
                    }
                }
            }
            Err(e) => {
                self.warn(format!("Áudio do jogo desativado: {e}"));
                None
            }
        }
    }

    /// Aplica pedidos pendentes de microfone e devolve um pedido de loopback, se houver.
    fn poll_controls(&mut self) -> Option<Option<AudioTarget>> {
        if !self.retired_mics.is_empty() {
            self.reap_mics(false);
        }
        if !self.shared.dirty.swap(false, Ordering::SeqCst) {
            return None;
        }
        let (loopback, microphone) = {
            let mut r = self.shared.requests();
            (r.loopback.take(), r.microphone.take())
        };
        if let Some(mic) = microphone {
            self.apply_mic(mic);
        }
        loopback
    }

    fn apply_mic(&mut self, mic: Option<MicConfig>) {
        if let Some(old) = self.mic.take() {
            old.stop.store(true, Ordering::SeqCst);
            self.retired_mics.push(old);
        }
        // Nova geração: a thread antiga para de escrever na fila e no estado.
        let generation = self.shared.mic_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.shared.mic_fifo().clear();
        let Some(mic) = mic else {
            self.shared.set_mic_state(generation, MicState::Off);
            tracing::info!("microfone desligado");
            return;
        };
        self.shared.set_mic_state(generation, MicState::Starting);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let shared = Arc::clone(&self.shared);
        let warn = Arc::clone(&self.on_warning);
        let device = mic.device_id.clone();
        let spawned = std::thread::Builder::new()
            .name("vellcast-mic".into())
            .spawn(move || run_mic(device, generation, shared, thread_stop, warn));
        match spawned {
            Ok(thread) => self.mic = Some(MicHandle { stop, thread }),
            Err(e) => {
                self.shared.set_mic_state(generation, MicState::Unavailable);
                self.warn(format!("Microfone indisponível: {e}"));
            }
        }
    }

    /// Junta threads de microfone já encerradas (ou todas, com `all`).
    fn reap_mics(&mut self, all: bool) {
        let mut keep = Vec::new();
        for m in self.retired_mics.drain(..) {
            if all || m.thread.is_finished() {
                m.stop.store(true, Ordering::SeqCst);
                let _ = m.thread.join();
            } else {
                keep.push(m);
            }
        }
        self.retired_mics = keep;
    }

    fn stop_mics(&mut self) {
        if let Some(m) = self.mic.take() {
            self.retired_mics.push(m);
        }
        self.reap_mics(true);
    }

    fn run(&mut self, on_packet: PacketSink, ready: mpsc::SyncSender<Result<(), AudioError>>) {
        let _com = ComGuard::init();
        tracing::debug!("áudio: COM iniciado; criando encoder Opus");
        let encoder = match OpusEncoder::new() {
            Ok(e) => e,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        tracing::debug!("áudio: encoder Opus pronto; abrindo a captura");
        let mut current = match self.initial.loopback {
            Some(target) => match self.open_target(target) {
                Ok(v) => Some(v),
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            },
            None => None,
        };
        let _ = ready.send(Ok(()));
        tracing::info!(
            source = ?current.as_ref().map(|(s, _)| s),
            converted = current.as_ref().is_some_and(|(_, st)| st.converter.is_some()),
            "captura de áudio iniciada"
        );
        if let Some(mic) = self.initial.microphone.take() {
            self.apply_mic(Some(mic));
        }

        let mut emitter = Emitter {
            encoder,
            on_packet,
            on_warning: Arc::clone(&self.on_warning),
            shared: Arc::clone(&self.shared),
            mixed: Vec::with_capacity(FRAME_SAMPLES * 2),
            mic: vec![0.0; FRAME_SAMPLES * 2],
            encode_failed: false,
        };
        let mut assembler = FrameAssembler::new();
        'outer: loop {
            self.shared
                .loopback
                .store(current.is_some(), Ordering::Relaxed);
            let exit = match current.as_mut() {
                Some((source, stream)) => {
                    let source = source.clone();
                    self.pump(stream, &source, &mut assembler, &mut emitter)
                }
                None => self.pump_silence(&mut assembler, &mut emitter),
            };
            match exit {
                Exit::Stopped => break,
                Exit::Reconfigure(target) => {
                    // Fecha a origem anterior antes de abrir a nova; a linha do
                    // tempo continua (lacunas viram silêncio).
                    drop(current.take());
                    self.shared.active.store(false, Ordering::Relaxed);
                    tracing::info!(?target, "trocando a origem do áudio");
                    current = target.and_then(|t| self.open_live(t));
                }
                Exit::Restart(reason) => {
                    self.shared.active.store(false, Ordering::Relaxed);
                    tracing::warn!(%reason, "reabrindo a captura de áudio");
                    let Some((source, stream)) = current.take() else {
                        continue;
                    };
                    drop(stream);
                    let mut attempt = 0usize;
                    loop {
                        let delay = RESTART_BACKOFF[attempt.min(RESTART_BACKOFF.len() - 1)];
                        attempt += 1;
                        if self.sleep_unless_stopped(delay) {
                            break 'outer;
                        }
                        if let Some(target) = self.poll_controls() {
                            current = target.and_then(|t| self.open_live(t));
                            continue 'outer;
                        }
                        match Stream::open(&source) {
                            Ok(s) => {
                                current = Some((source, s));
                                break;
                            }
                            Err(e) => {
                                if attempt == 3 {
                                    self.warn(format!("Áudio indisponível: {e}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.stop_mics();
        self.shared.active.store(false, Ordering::Relaxed);
        self.shared.loopback.store(false, Ordering::Relaxed);
    }

    /// Dorme em fatias curtas; retorna `true` se a captura foi parada.
    fn sleep_unless_stopped(&self, total: Duration) -> bool {
        let end = Instant::now() + total;
        while Instant::now() < end {
            if self.stopping() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.stopping()
    }

    /// Sem loopback: completa a linha do tempo com silêncio (e o microfone) a cada 20 ms.
    fn pump_silence(&mut self, assembler: &mut FrameAssembler, emitter: &mut Emitter) -> Exit {
        assembler.start_at(self.clock.now_pts().saturating_sub(SILENCE_ONLY_LAG_US));
        loop {
            if self.stopping() {
                return Exit::Stopped;
            }
            if let Some(target) = self.poll_controls() {
                return Exit::Reconfigure(target);
            }
            std::thread::sleep(Duration::from_millis(u64::from(WAIT_MS)));
            let now = self.clock.now_pts();
            assembler.fill_silence_until(now.saturating_sub(SILENCE_ONLY_LAG_US), &mut |f, p| {
                emitter.emit(f, p)
            });
        }
    }

    fn pump(
        &mut self,
        stream: &mut Stream,
        source: &Source,
        assembler: &mut FrameAssembler,
        emitter: &mut Emitter,
    ) -> Exit {
        let mut samples: Vec<f32> = Vec::with_capacity(48_000 / 10 * 2);
        let mut last_device_check = Instant::now();
        loop {
            if self.stopping() {
                return Exit::Stopped;
            }
            if let Some(target) = self.poll_controls() {
                return Exit::Reconfigure(target);
            }
            stream.event.wait(WAIT_MS);

            let gain = self.shared.gain();
            let clock = &self.clock;
            let shared = &self.shared;
            let drained = stream.drain(&mut samples, gain, false, &mut |data, qpc_hns| {
                let pts = if qpc_hns > 0 {
                    clock.pts_from_hns(qpc_hns)
                } else {
                    clock.now_pts()
                };
                assembler.push(data, pts, &mut |f, p| emitter.emit(f, p));
                shared.active.store(true, Ordering::Relaxed);
            });
            if let Err(e) = drained {
                return restart_reason(e);
            }

            // Mantém o fluxo contínuo quando o dispositivo não entrega pacotes (silêncio).
            let now = self.clock.now_pts();
            assembler.fill_silence_until(now.saturating_sub(SILENCE_FILL_LAG_US), &mut |f, p| {
                emitter.emit(f, p)
            });

            if *source == Source::Endpoint && last_device_check.elapsed() >= DEVICE_CHECK_INTERVAL {
                last_device_check = Instant::now();
                if let Some(current) = current_default_device_id()
                    && stream.device_id.as_deref() != Some(current.as_str())
                {
                    return Exit::Restart("dispositivo de saída padrão mudou".into());
                }
            }
        }
    }
}

/// Thread de um microfone: abre o dispositivo, enche a fila e reabre com
/// backoff se ele cair. Desiste (estado `Unavailable` + aviso) se o dispositivo
/// sumir; a transmissão continua sem o microfone.
fn run_mic(
    device: Option<String>,
    generation: u64,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    warn: Arc<WarningSink>,
) {
    let _com = ComGuard::init();
    let current = || {
        !stop.load(Ordering::Relaxed)
            && !shared.stop.load(Ordering::Relaxed)
            && shared.mic_generation.load(Ordering::SeqCst) == generation
    };
    let source = Source::Microphone(device);
    let mut samples: Vec<f32> = Vec::with_capacity(48_000 / 10 * 2);
    let mut failures = 0usize;
    while current() {
        let last_error = match Stream::open(&source) {
            Ok(mut stream) => {
                tracing::info!(
                    ?source,
                    converted = stream.converter.is_some(),
                    "microfone aberto"
                );
                failures = 0;
                shared.set_mic_state(generation, MicState::Active);
                match pump_mic(&mut stream, &mut samples, &shared, generation, &current) {
                    None => return,
                    Some(reason) => {
                        tracing::warn!(%reason, "microfone caiu; reabrindo");
                        shared.set_mic_state(generation, MicState::Starting);
                        if shared.mic_generation.load(Ordering::SeqCst) == generation {
                            shared.mic_fifo().clear();
                        }
                        reason
                    }
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "não foi possível abrir o microfone");
                e.to_string()
            }
        };
        failures += 1;
        if failures > MIC_MAX_RETRIES {
            if current() {
                shared.set_mic_state(generation, MicState::Unavailable);
                warn(format!(
                    "Microfone indisponível ({last_error}). A transmissão continua sem o microfone."
                ));
            }
            return;
        }
        let end = Instant::now() + RESTART_BACKOFF[(failures - 1).min(RESTART_BACKOFF.len() - 1)];
        while Instant::now() < end {
            if !current() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Laço de leitura do microfone. `None` = parado; `Some(motivo)` = dispositivo caiu.
fn pump_mic(
    stream: &mut Stream,
    samples: &mut Vec<f32>,
    shared: &Shared,
    generation: u64,
    current: &dyn Fn() -> bool,
) -> Option<String> {
    loop {
        if !current() {
            return None;
        }
        stream.event.wait(WAIT_MS);
        // Mudo: os pacotes são liberados sem conversão e nada entra na fila.
        let muted = shared.mic_muted.load(Ordering::Relaxed);
        let result = stream.drain(samples, 1.0, muted, &mut |data, _| {
            let mut fifo: std::sync::MutexGuard<'_, MicFifo> = shared.mic_fifo();
            if shared.mic_generation.load(Ordering::SeqCst) == generation {
                fifo.push(data);
            }
        });
        if let Err(e) = result {
            return Some(match restart_reason(e) {
                Exit::Restart(reason) => reason,
                _ => "falha no dispositivo".into(),
            });
        }
    }
}

fn restart_reason(e: windows_core::Error) -> Exit {
    if e.code() == AUDCLNT_E_DEVICE_INVALIDATED {
        Exit::Restart("dispositivo de áudio invalidado".into())
    } else {
        Exit::Restart(format!("{} ({:#010x})", e.message(), e.code().0))
    }
}
