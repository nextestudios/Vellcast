//! Captura WASAPI no Windows.
//!
//! Uma thread dedicada (COM MTA) abre o cliente de áudio, espera pelo evento
//! do WASAPI (sem polling ativo), converte, monta blocos de 20 ms e codifica
//! em Opus. Falhas do dispositivo reabrem a captura com backoff.

use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
    AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_PARAMS_0,
    AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK, AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS,
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    PROCESS_LOOPBACK_MODE, PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
    PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVE_FORMAT_PCM, WAVEFORMATEX, WAVEFORMATEXTENSIBLE, eConsole, eRender,
};
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{
    BLOB, CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
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
use crate::opus_encoder::OpusEncoder;
use crate::process::{ProcessEntry, choose_exclusion};
use crate::{
    AudioError, AudioPacket, AudioTarget, CHANNELS, PacketSink, SAMPLE_RATE, Shared, WarningSink,
};

/// Espera máxima pelo evento do WASAPI; também define a frequência de checagem de parada.
const WAIT_MS: u32 = 20;
/// Duração do buffer compartilhado pedida ao WASAPI (100 ms, em unidades de 100 ns).
const BUFFER_HNS: i64 = 1_000_000;
/// Durante silêncio sem pacotes, a linha do tempo é completada até `agora - LAG`.
/// A folga evita cortar dados reais que chegam com a latência do dispositivo.
const SILENCE_FILL_LAG_US: u64 = 100_000;
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
    target: AudioTarget,
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
            let mut capture = CaptureThread {
                target,
                clock: QpcClock::new(origin_qpc),
                on_packet,
                on_warning,
                shared: thread_shared,
            };
            capture.run(ready_tx);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    ProcessLoopback {
        pid: u32,
        mode: PROCESS_LOOPBACK_MODE,
    },
    /// Loopback clássico do dispositivo de saída padrão.
    Endpoint,
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
    // Parâmetros e PROPVARIANT ficam no heap: se a ativação passar do prazo, eles
    // são vazados (o Windows ainda pode lê-los) em vez de apontar para a pilha.
    let params = Box::new(AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: mode,
            },
        },
    });
    let mut prop = Box::new(PROPVARIANT::default());
    // SAFETY: monta um PROPVARIANT VT_BLOB que aponta para `params`, que vive
    // até o fim desta função (a ativação é concluída antes do retorno). O
    // PROPVARIANT não é liberado com `PropVariantClear`, pois não é dono do blob.
    unsafe {
        let inner = &mut *prop.Anonymous.Anonymous;
        inner.vt = VT_BLOB;
        inner.Anonymous.blob = BLOB {
            cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
            pBlobData: std::ptr::from_ref::<AUDIOCLIENT_ACTIVATION_PARAMS>(&params)
                .cast_mut()
                .cast::<u8>(),
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
        // A operação pode concluir depois: mantém `params`, o evento e o handler
        // vivos para sempre em vez de liberar memória que o Windows ainda vai ler.
        std::mem::forget(done);
        std::mem::forget(handler);
        std::mem::forget(operation);
        Box::leak(params);
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
    fn open(source: Source) -> Result<Self, AudioError> {
        let loopback = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
        let convert = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
        let desired = desired_format();

        let (client, device_id) = match source {
            Source::ProcessLoopback { pid, mode } => (activate_process_loopback(pid, mode)?, None),
            Source::Endpoint => {
                // SAFETY: COM inicializado nesta thread por `ComGuard`.
                let enumerator: IMMDeviceEnumerator =
                    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                        .map_err(wasapi_err)?;
                // SAFETY: enumerador válido.
                let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
                    .map_err(|e| AudioError::NoDevice(e.message().to_string()))?;
                let id = default_device_id(&enumerator);
                // SAFETY: dispositivo válido; sem parâmetros de ativação.
                let client: IAudioClient =
                    unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(wasapi_err)?;
                (client, id)
            }
        };

        tracing::debug!(?source, "áudio: inicializando o cliente");
        // Primeiro tenta deixar o Windows converter para 48 kHz estéreo f32.
        // SAFETY: `desired` vive durante a chamada; cliente ainda não inicializado.
        let auto = unsafe {
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                loopback | convert,
                BUFFER_HNS,
                0,
                &desired,
                None,
            )
        };
        let (input, converter) = match auto {
            Ok(()) => (native_input(), None),
            Err(first) if source == Source::Endpoint => {
                // Sem conversão automática: usa o formato de mixagem e converte aqui.
                // SAFETY: o WASAPI aloca o formato; liberado com `CoTaskMemFree` abaixo.
                let mix = unsafe { client.GetMixFormat() }.map_err(wasapi_err)?;
                // SAFETY: `mix` aponta para um WAVEFORMATEX válido devolvido pelo WASAPI.
                let parsed = unsafe { parse_format(mix) };
                // SAFETY: `mix` vive durante a chamada.
                let init = unsafe {
                    client.Initialize(AUDCLNT_SHAREMODE_SHARED, loopback, BUFFER_HNS, 0, mix, None)
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
                        loopback,
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
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: cliente válido; parar antes de liberar o evento associado.
        let _ = unsafe { self.client.Stop() };
    }
}

fn default_device_id(enumerator: &IMMDeviceEnumerator) -> Option<String> {
    // SAFETY: enumerador válido.
    let device = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok()?;
    // SAFETY: dispositivo válido; a string é alocada pelo COM e liberada abaixo.
    let id = unsafe { device.GetId() }.ok()?;
    // SAFETY: `id` é uma string UTF-16 terminada em zero válida.
    let text = unsafe { id.to_string() }.ok();
    // SAFETY: memória alocada pelo COM, liberada uma vez.
    unsafe { CoTaskMemFree(Some(id.0.cast_const().cast())) };
    text
}

fn current_default_device_id() -> Option<String> {
    // SAFETY: COM inicializado nesta thread.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.ok()?;
    default_device_id(&enumerator)
}

/// Motivo de saída do laço de um `Stream`.
enum Exit {
    Stopped,
    Restart(String),
}

struct CaptureThread {
    target: AudioTarget,
    clock: QpcClock,
    on_packet: PacketSink,
    on_warning: WarningSink,
    shared: Arc<Shared>,
}

impl CaptureThread {
    fn stopping(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }

    /// Decide a origem e abre o primeiro `Stream`, com fallback no modo sistema.
    fn open_initial(&self) -> Result<(Source, Stream), AudioError> {
        match self.target {
            AudioTarget::Process(pid) => {
                let source = Source::ProcessLoopback {
                    pid,
                    mode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                };
                Ok((source, Stream::open(source)?))
            }
            AudioTarget::System => {
                // SAFETY: sem pré-condições.
                let own = unsafe { GetCurrentProcessId() };
                let processes = list_processes();
                tracing::debug!(count = processes.len(), "áudio: processos listados");
                let exclusion = choose_exclusion(&processes, own);
                tracing::debug!(pid = exclusion.pid, "áudio: árvore de processos excluída");
                if let Some(w) = &exclusion.warning {
                    (self.on_warning)(w.clone());
                }
                let source = Source::ProcessLoopback {
                    pid: exclusion.pid,
                    mode: PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE,
                };
                match Stream::open(source) {
                    Ok(stream) => Ok((source, stream)),
                    Err(e) => {
                        tracing::warn!(error = %e, "loopback por processo indisponível; usando o dispositivo padrão");
                        (self.on_warning)(
                            "Capturando todo o áudio do sistema: o áudio do Discord (voz) pode ser transmitido."
                                .into(),
                        );
                        Ok((Source::Endpoint, Stream::open(Source::Endpoint)?))
                    }
                }
            }
        }
    }

    fn run(&mut self, ready: mpsc::SyncSender<Result<(), AudioError>>) {
        let _com = ComGuard::init();
        tracing::debug!("áudio: COM iniciado; criando encoder Opus");
        let mut encoder = match OpusEncoder::new() {
            Ok(e) => e,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        tracing::debug!("áudio: encoder Opus pronto; abrindo a captura");
        let (source, mut stream) = match self.open_initial() {
            Ok(v) => v,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        let _ = ready.send(Ok(()));
        tracing::info!(
            ?source,
            converted = stream.converter.is_some(),
            "captura de áudio iniciada"
        );

        let mut assembler = FrameAssembler::new();
        let mut attempt = 0usize;
        loop {
            match self.pump(&mut stream, &mut assembler, &mut encoder, source) {
                Exit::Stopped => break,
                Exit::Restart(reason) => {
                    self.shared.active.store(false, Ordering::Relaxed);
                    tracing::warn!(%reason, "reabrindo a captura de áudio");
                    drop(stream);
                    loop {
                        let delay = RESTART_BACKOFF[attempt.min(RESTART_BACKOFF.len() - 1)];
                        attempt += 1;
                        if self.sleep_unless_stopped(delay) {
                            return;
                        }
                        match Stream::open(source) {
                            Ok(s) => {
                                stream = s;
                                attempt = 0;
                                break;
                            }
                            Err(e) => {
                                if attempt == 3 {
                                    (self.on_warning)(format!("Áudio indisponível: {e}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.shared.active.store(false, Ordering::Relaxed);
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

    fn pump(
        &mut self,
        stream: &mut Stream,
        assembler: &mut FrameAssembler,
        encoder: &mut OpusEncoder,
        source: Source,
    ) -> Exit {
        let shared = Arc::clone(&self.shared);
        let on_packet = &mut self.on_packet;
        let on_warning = &self.on_warning;
        let mut encode_failed = false;
        let mut emit = |frame: &[f32], pts_us: u64| match encoder.encode(frame) {
            Ok(data) => {
                shared.packets.fetch_add(1, Ordering::Relaxed);
                on_packet(AudioPacket { data, pts_us });
            }
            Err(e) => {
                if !encode_failed {
                    encode_failed = true;
                    on_warning(format!("Falha ao codificar áudio: {e}"));
                }
            }
        };

        let mut samples: Vec<f32> = Vec::with_capacity(48_000 / 10 * 2);
        let mut last_device_check = Instant::now();
        loop {
            if shared.stop.load(Ordering::Relaxed) {
                return Exit::Stopped;
            }
            stream.event.wait(WAIT_MS);

            loop {
                // SAFETY: cliente de captura válido.
                let next = match unsafe { stream.capture.GetNextPacketSize() } {
                    Ok(n) => n,
                    Err(e) => return restart_reason(e),
                };
                if next == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                let mut qpc_hns = 0u64;
                // SAFETY: ponteiros de saída locais; o buffer é válido até `ReleaseBuffer`.
                if let Err(e) = unsafe {
                    stream.capture.GetBuffer(
                        &mut data,
                        &mut frames,
                        &mut flags,
                        None,
                        Some(&mut qpc_hns),
                    )
                } {
                    return restart_reason(e);
                }
                let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null();
                let len = frames as usize * stream.frame_bytes;
                let bytes: &[u8] = if silent {
                    &[]
                } else {
                    // SAFETY: o WASAPI garante `frames * nBlockAlign` bytes válidos em `data`
                    // até `ReleaseBuffer`, chamado logo após o uso.
                    unsafe { std::slice::from_raw_parts(data, len) }
                };
                samples.clear();
                let gain = shared.gain();
                match stream.converter.as_mut() {
                    Some(conv) => conv.convert(bytes, frames as usize, silent, gain, &mut samples),
                    None if silent => samples.resize(frames as usize * 2, 0.0),
                    None => samples.extend(bytes.chunks_exact(4).map(|b| {
                        (f32::from_le_bytes([b[0], b[1], b[2], b[3]]) * gain).clamp(-1.0, 1.0)
                    })),
                }
                // SAFETY: libera exatamente o que foi obtido em `GetBuffer`.
                if let Err(e) = unsafe { stream.capture.ReleaseBuffer(frames) } {
                    return restart_reason(e);
                }
                let pts = if qpc_hns > 0 {
                    self.clock.pts_from_hns(qpc_hns)
                } else {
                    self.clock.now_pts()
                };
                assembler.push(&samples, pts, &mut emit);
                shared.active.store(true, Ordering::Relaxed);
            }

            // Mantém o fluxo contínuo quando o dispositivo não entrega pacotes (silêncio).
            let now = self.clock.now_pts();
            assembler.fill_silence_until(now.saturating_sub(SILENCE_FILL_LAG_US), &mut emit);

            if source == Source::Endpoint && last_device_check.elapsed() >= DEVICE_CHECK_INTERVAL {
                last_device_check = Instant::now();
                if let Some(current) = current_default_device_id() {
                    if stream.device_id.as_deref() != Some(current.as_str()) {
                        return Exit::Restart("dispositivo de saída padrão mudou".into());
                    }
                }
            }
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
