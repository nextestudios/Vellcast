//! Teste de ponta a ponta no Windows: transmite uma fonte real ao relay pelo
//! mesmo caminho do app desktop (`StreamSession`).
//!
//! ```text
//! set VELLCAST_PUBLISHER_TOKEN=<token de transmissor>
//! cargo run --release -p vellcast-core --example stream_to_relay -- ^
//!     ws://127.0.0.1:8788/ws <fonte|monitor> <segundos> [fps] [kbps] [audio: off|app|system]
//! ```
//!
//! Imprime eventos e métricas (uma linha `STATS {...}` a cada 5 s e `FINAL {...}`).
//! Sai com código 1 se não conectar ao relay ou se nenhum quadro for codificado.
//!
//! Com `VELLCAST_E2E_LIVE=1`, exercita os controles rápidos (issue #5) pelo mesmo
//! caminho do app: em 1/3 do tempo muda bitrate e preferência de conteúdo e liga o
//! microfone padrão (sem microfone, o estado vira indisponível e a transmissão
//! segue); na metade, reinicia o vídeo em 720p (`VideoRestarted`). Sai com código 1
//! se o reinício não acontecer.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use vellcast_core::{
    Adaptation, AudioMode, EncoderPreference, LiveChange, MicrophoneConfig, Quality, SessionConfig,
    SessionEvent, SourceKind, StreamSession, list_audio_inputs, list_encoders, list_sources,
};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    if let Err(err) = run() {
        eprintln!("ERRO: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 3 {
        return Err(
            "uso: stream_to_relay <ws-url> <indice|monitor> <segundos> [fps] [kbps] [off|app|system] [balanced|competitive|off]"
                .into(),
        );
    }
    let url = args[0].clone();
    let seconds: u64 = args[2].parse().map_err(|_| "segundos inválidos")?;
    let fps: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(30);
    let kbps: u32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(4000);
    let audio = match args.get(5).map(String::as_str) {
        Some("app") => AudioMode::Application,
        Some("system") => AudioMode::System,
        _ => AudioMode::Off,
    };
    let adaptation = match args.get(6).map(String::as_str) {
        Some("competitive") => vellcast_core::Adaptation::Competitive,
        Some("off") => vellcast_core::Adaptation::Off,
        _ => vellcast_core::Adaptation::Balanced,
    };
    let token =
        std::env::var("VELLCAST_PUBLISHER_TOKEN").map_err(|_| "defina VELLCAST_PUBLISHER_TOKEN")?;

    let sources = list_sources().map_err(|e| e.to_string())?;
    for (i, s) in sources.iter().enumerate() {
        println!(
            "FONTE [{i}] {:?} {}x{} {:?}",
            s.kind, s.width, s.height, s.title
        );
    }
    match list_encoders() {
        Ok(list) => {
            for e in list {
                println!("ENCODER {} (hardware: {})", e.name, e.hardware);
            }
        }
        Err(e) => println!("ENCODER erro: {e}"),
    }
    let source = if args[1] == "monitor" {
        sources.iter().find(|s| s.kind == SourceKind::Monitor)
    } else {
        args[1].parse::<usize>().ok().and_then(|i| sources.get(i))
    }
    .cloned()
    .ok_or("fonte não encontrada")?;
    println!("TRANSMITINDO {:?} {:?}", source.kind, source.title);

    let live = std::env::var("VELLCAST_E2E_LIVE").as_deref() == Ok("1");
    if live {
        match list_audio_inputs() {
            Ok(list) if list.is_empty() => println!("MICROFONES nenhum"),
            Ok(list) => {
                for m in list {
                    println!("MICROFONE {:?} (padrão: {})", m.name, m.is_default);
                }
            }
            Err(e) => println!("MICROFONES erro: {e}"),
        }
    }

    let quality = Quality {
        max_width: 1920,
        max_height: 1080,
        fps,
        bitrate_kbps: kbps,
    };
    let log = Arc::new(Mutex::new(Vec::<SessionEvent>::new()));
    let sink_log = Arc::clone(&log);
    let session = StreamSession::start(
        SessionConfig {
            source: source.clone(),
            quality,
            audio,
            encoder: EncoderPreference::Auto,
            relay_ws_url: url,
            publisher_token: token,
            adaptation,
            display_name: None,
        },
        Box::new(move |event| {
            println!("EVENTO {event:?}");
            if let Ok(mut l) = sink_log.lock() {
                l.push(event);
            }
        }),
    )
    .map_err(|e| format!("StreamSession::start: {e}"))?;

    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds);
    let mut tick = 0u64;
    let (mut changed, mut restarted) = (false, false);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_secs(1));
        tick += 1;
        let elapsed = start.elapsed().as_secs();
        if live && !changed && elapsed >= seconds / 3 {
            changed = true;
            println!("LIVE bitrate {} kbps, nitidez, microfone padrão", kbps / 2);
            session.apply(LiveChange::Bitrate(kbps / 2));
            session.apply(LiveChange::Adaptation(Adaptation::Clarity));
            session.apply(LiveChange::Microphone(Some(MicrophoneConfig::default())));
        }
        if live && !restarted && elapsed >= seconds / 2 {
            restarted = true;
            println!("LIVE reinício do vídeo em 720p");
            session.apply(LiveChange::Video {
                source: source.clone(),
                quality: Quality {
                    max_width: 1280,
                    max_height: 720,
                    ..quality
                },
            });
        }
        if !tick.is_multiple_of(5) {
            continue;
        }
        let s = session.stats();
        println!(
            "STATS {{\"t\":{},\"captured_fps\":{:.1},\"encoded_fps\":{:.1},\"bitrate_kbps\":{:.0},\"encode_ms\":{:.2},\"dropped\":{},\"net_dropped\":{},\"audio\":{},\"reconnects\":{},\"target_kbps\":{},\"target_fps\":{},\"game_audio\":{},\"mic\":\"{:?}\",\"live_bitrate\":\"{:?}\"}}",
            elapsed,
            s.captured_fps,
            s.encoded_fps,
            s.bitrate_kbps,
            s.encode_ms_avg,
            s.frames_dropped,
            s.network_dropped,
            s.audio_active,
            s.reconnects,
            s.target_bitrate_kbps,
            s.target_fps,
            s.game_audio,
            s.microphone,
            s.live_bitrate
        );
        let stopped = log
            .lock()
            .map(|l| l.iter().any(|e| matches!(e, SessionEvent::Stopped { .. })))
            .unwrap_or(false);
        if stopped {
            break;
        }
    }
    let s = session.stats();
    session.stop();
    let events = log.lock().map(|l| l.clone()).unwrap_or_default();
    let connected = events.iter().any(|e| matches!(e, SessionEvent::Connected));
    let video_restarted = events
        .iter()
        .any(|e| matches!(e, SessionEvent::VideoRestarted { height, .. } if *height <= 720));
    println!(
        "FINAL {{\"seconds\":{},\"connected\":{connected},\"encoded_fps\":{:.1},\"bitrate_kbps\":{:.0},\"audio\":{},\"video_restarted\":{video_restarted},\"mic\":\"{:?}\"}}",
        start.elapsed().as_secs(),
        s.encoded_fps,
        s.bitrate_kbps,
        s.audio_active,
        s.microphone
    );
    if !connected {
        return Err("a sessão nunca conectou ao relay".into());
    }
    if s.encoded_fps <= 0.0 && s.bitrate_kbps <= 0.0 {
        return Err("nenhum quadro codificado".into());
    }
    if live && !video_restarted {
        return Err("o reinício do vídeo pedido durante a transmissão não aconteceu".into());
    }
    Ok(())
}
