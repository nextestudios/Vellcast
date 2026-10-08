//! Teste de captura real; requer Windows com dispositivo de saída de áudio.
//! Executar com: `cargo test -p vellcast-audio -- --ignored`
//! (de preferência com algum áudio tocando).

#![cfg(windows)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use vellcast_audio::{AudioCapture, AudioTarget, FRAME_DURATION_US, qpc_now};

#[test]
#[ignore = "requer dispositivo de áudio"]
fn captures_system_audio_for_one_second() {
    let pts = Arc::new(Mutex::new(Vec::<u64>::new()));
    let sink = Arc::clone(&pts);
    let capture = AudioCapture::start(
        AudioTarget::System,
        qpc_now().expect("QPC"),
        Box::new(move |p| {
            assert!(!p.data.is_empty());
            sink.lock().expect("lock").push(p.pts_us);
        }),
        Box::new(|w| eprintln!("aviso: {w}")),
    )
    .expect("captura iniciada");
    std::thread::sleep(Duration::from_millis(1200));
    let stats = capture.stats();
    capture.stop();

    let pts = pts.lock().expect("lock");
    eprintln!("{} pacotes, {stats:?}", pts.len());
    // Com áudio tocando, ~50 pacotes por segundo.
    for w in pts.windows(2) {
        assert_eq!(w[1] - w[0], FRAME_DURATION_US, "PTS contíguos");
    }
}

#[test]
#[ignore = "requer Windows 10 2004+"]
fn process_loopback_for_own_process_starts() {
    let pid = std::process::id();
    let capture = AudioCapture::start(
        AudioTarget::Process(pid),
        qpc_now().expect("QPC"),
        Box::new(|_| {}),
        Box::new(|w| eprintln!("aviso: {w}")),
    )
    .expect("loopback por processo disponível");
    std::thread::sleep(Duration::from_millis(300));
    capture.stop();
}
