//! Teste manual no Windows: captura uma fonte e grava H.264 bruto em arquivo.
//!
//! ```text
//! cargo run --release -p vellcast-core --example capture_to_file -- list
//! cargo run --release -p vellcast-core --example capture_to_file -- 0 10 saida.h264
//! ffplay saida.h264        # ou abra no VLC
//! ```
//!
//! Não usa o relay: valida isoladamente captura WGC → NV12 na GPU → encoder.

#[cfg(windows)]
fn main() {
    if let Err(err) = windows_main::run() {
        eprintln!("erro: {err}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("este exemplo só funciona no Windows");
}

#[cfg(windows)]
mod windows_main {
    use std::io::Write;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use vellcast_capture::slot::LatestSlot;
    use vellcast_capture::{CaptureOptions, Capturer, D3dDevice, list_sources};
    use vellcast_encoder::sizing::fit_dimensions;
    use vellcast_encoder::{EncoderConfig, H264Encoder, init_thread, list_encoders};

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let sources = list_sources()?;
        if args.first().map(String::as_str) != Some("list") && args.len() < 3 {
            eprintln!(
                "uso: capture_to_file list | <índice> <segundos> <arquivo.h264> [fps] [kbps]"
            );
        }
        if args.len() < 3 {
            println!("Fontes:");
            for (i, s) in sources.iter().enumerate() {
                println!(
                    "  [{i}] {}{} {}x{} {}",
                    if s.likely_game { "🎮 " } else { "" },
                    s.title,
                    s.width,
                    s.height,
                    s.process_name.as_deref().unwrap_or("")
                );
            }
            println!("Encoders H.264:");
            for e in list_encoders()? {
                println!("  {} ({:?}, hardware: {})", e.name, e.vendor, e.hardware);
            }
            return Ok(());
        }
        let index: usize = args[0].parse()?;
        let seconds: u64 = args[1].parse()?;
        let path = &args[2];
        let fps: u32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(60);
        let kbps: u32 = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(8000);
        let source = sources.get(index).ok_or("índice inválido")?;

        init_thread();
        let device = D3dDevice::new()?;
        let (w, h) = fit_dimensions(source.width, source.height, 1920, 1080);
        let mut encoder = H264Encoder::new(
            &device.device,
            EncoderConfig {
                width: w,
                height: h,
                fps,
                bitrate_kbps: kbps,
                keyframe_interval_s: 2,
                preferred: None,
            },
        )?;
        println!(
            "Encoder: {} — saída {w}x{h} @ {fps} fps, {kbps} kbps",
            encoder.descriptor().name
        );

        let slot = Arc::new(LatestSlot::new());
        let closed = Arc::new(AtomicBool::new(false));
        let closed_flag = Arc::clone(&closed);
        let capturer = Capturer::start(
            &device,
            source.target,
            CaptureOptions { fps, cursor: true },
            Arc::clone(&slot),
            Box::new(move || closed_flag.store(true, Ordering::SeqCst)),
        )?;

        let mut file = std::fs::File::create(path)?;
        let mut out = Vec::new();
        let (mut frames, mut keyframes, mut bytes, mut busy) = (0u64, 0u64, 0u64, 0u64);
        let start = Instant::now();
        let deadline = start + Duration::from_secs(seconds);
        let mut first = true;
        while Instant::now() < deadline && !closed.load(Ordering::SeqCst) {
            if let Some(frame) = slot.take(Duration::from_millis(5)) {
                if encoder.ready_for_input() {
                    let pts = (start.elapsed().as_nanos() / 100) as i64;
                    let outcome = encoder.submit(
                        &frame.texture,
                        frame.width,
                        frame.height,
                        pts,
                        first,
                        &mut out,
                    )?;
                    first = false;
                    if outcome == vellcast_encoder::SubmitOutcome::Busy {
                        busy += 1;
                    }
                } else {
                    busy += 1;
                }
            }
            encoder.poll(&mut out)?;
            for f in out.drain(..) {
                frames += 1;
                keyframes += u64::from(f.keyframe);
                bytes += f.data.len() as u64;
                file.write_all(&f.data)?;
            }
        }
        let stats = capturer.stats();
        capturer.stop();
        let secs = start.elapsed().as_secs_f64();
        println!(
            "{frames} quadros ({:.1} fps), {keyframes} keyframes, {:.0} kbps, {busy} descartados pelo encoder; captura: {} recebidos, {} acima da taxa, {} substituídos",
            frames as f64 / secs,
            bytes as f64 * 8.0 / 1000.0 / secs,
            stats.arrived,
            stats.paced_out,
            stats.replaced
        );
        if closed.load(Ordering::SeqCst) {
            println!("a fonte foi fechada durante a captura");
        }
        println!(
            "codec: {}",
            encoder
                .codec_string()
                .unwrap_or_else(|| "desconhecido".into())
        );
        Ok(())
    }
}
