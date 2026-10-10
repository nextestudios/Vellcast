//! Teste manual da captura de áudio no Windows.
//!
//! Uso:
//!   cargo run -p vellcast-audio --example capture_audio -- system 10 saida.opus
//!   cargo run -p vellcast-audio --example capture_audio -- <pid> 10 saida.opus
//!   cargo run -p vellcast-audio --example capture_audio -- mic 10 microfone.opus
//!   cargo run -p vellcast-audio --example capture_audio -- system+mic 10 mix.opus
//!   cargo run -p vellcast-audio --example capture_audio -- devices
//!
//! `mic` grava só o microfone padrão (linha do tempo de silêncio + microfone);
//! `system+mic` mistura o áudio do sistema com o microfone. Sem microfone, o
//! estado termina `Unavailable` e o arquivo sai só com silêncio (sem falhar).
//!
//! Grava um arquivo Ogg Opus que pode ser ouvido no VLC, no ffplay ou no navegador,
//! e imprime estatísticas de PTS (intervalos e lacunas).

#[cfg(windows)]
mod ogg {
    //! Escritor Ogg Opus mínimo (RFC 7845), só para este exemplo.
    use std::io::Write;

    const PRE_SKIP: u16 = 312;

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0u32;
        for &b in data {
            crc ^= u32::from(b) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04C1_1DB7
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    pub struct Writer<W: Write> {
        out: W,
        serial: u32,
        sequence: u32,
        granule: u64,
    }

    impl<W: Write> Writer<W> {
        pub fn new(out: W) -> std::io::Result<Self> {
            let mut w = Self {
                out,
                serial: 0x5643_4153,
                sequence: 0,
                granule: 0,
            };
            let mut head = b"OpusHead".to_vec();
            head.push(1);
            head.push(2);
            head.extend_from_slice(&PRE_SKIP.to_le_bytes());
            head.extend_from_slice(&48_000u32.to_le_bytes());
            head.extend_from_slice(&0i16.to_le_bytes());
            head.push(0);
            w.page(&head, 0x02, 0)?;
            let vendor = b"vellcast";
            let mut tags = b"OpusTags".to_vec();
            tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
            tags.extend_from_slice(vendor);
            tags.extend_from_slice(&0u32.to_le_bytes());
            w.page(&tags, 0, 0)?;
            Ok(w)
        }

        pub fn packet(&mut self, data: &[u8], last: bool) -> std::io::Result<()> {
            self.granule += 960;
            let granule = self.granule;
            self.page(data, if last { 0x04 } else { 0 }, granule)
        }

        fn page(&mut self, data: &[u8], flags: u8, granule: u64) -> std::io::Result<()> {
            let mut lacing = vec![255u8; data.len() / 255];
            lacing.push((data.len() % 255) as u8);
            let mut page = b"OggS".to_vec();
            page.push(0);
            page.push(flags);
            page.extend_from_slice(&granule.to_le_bytes());
            page.extend_from_slice(&self.serial.to_le_bytes());
            page.extend_from_slice(&self.sequence.to_le_bytes());
            page.extend_from_slice(&0u32.to_le_bytes());
            page.push(lacing.len() as u8);
            page.extend_from_slice(&lacing);
            page.extend_from_slice(data);
            let crc = crc32(&page);
            page[22..26].copy_from_slice(&crc.to_le_bytes());
            self.sequence += 1;
            self.out.write_all(&page)
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::{Arc, Mutex};
    use vellcast_audio::{AudioCapture, AudioSetup, AudioTarget, MicConfig, qpc_now};

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("devices") {
        let list = vellcast_audio::list_input_devices()?;
        if list.is_empty() {
            println!("nenhum microfone encontrado");
        }
        for d in list {
            println!(
                "microfone: {:?} (padrão: {}) id={}",
                d.name, d.is_default, d.id
            );
        }
        return Ok(());
    }
    let mic = || Some(MicConfig::default());
    let setup = match args.first().map(String::as_str) {
        None | Some("system") => AudioSetup {
            loopback: Some(AudioTarget::System),
            microphone: None,
        },
        Some("mic") => AudioSetup {
            loopback: None,
            microphone: mic(),
        },
        Some("system+mic") => AudioSetup {
            loopback: Some(AudioTarget::System),
            microphone: mic(),
        },
        Some(pid) => AudioSetup {
            loopback: Some(AudioTarget::Process(pid.parse()?)),
            microphone: None,
        },
    };
    let target = setup.clone();
    let seconds: u64 = args.get(1).map_or(Ok(10), |s| s.parse())?;
    let path = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "vellcast-audio.opus".into());

    let packets = Arc::new(Mutex::new(Vec::<(Vec<u8>, u64)>::new()));
    let sink = Arc::clone(&packets);
    let capture = AudioCapture::start_with(
        setup,
        qpc_now().unwrap_or(0),
        Box::new(move |p| {
            if let Ok(mut v) = sink.lock() {
                v.push((p.data, p.pts_us));
            }
        }),
        Box::new(|w| eprintln!("aviso: {w}")),
    )?;
    println!("capturando {target:?} por {seconds} s…");
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    let stats = capture.stats();
    capture.stop();

    let packets = packets.lock().map_err(|_| "mutex envenenado")?;
    let mut writer = ogg::Writer::new(std::io::BufWriter::new(std::fs::File::create(&path)?))?;
    for (i, (data, _)) in packets.iter().enumerate() {
        writer.packet(data, i + 1 == packets.len())?;
    }
    let gaps: Vec<u64> = packets.windows(2).map(|w| w[1].1 - w[0].1).collect();
    let irregular = gaps.iter().filter(|&&g| g != 20_000).count();
    println!(
        "{} pacotes ({:?}), primeiro PTS {:?} µs, intervalos ≠ 20 ms: {irregular}, bytes médios {}",
        packets.len(),
        stats,
        packets.first().map(|p| p.1),
        packets.iter().map(|p| p.0.len()).sum::<usize>() / packets.len().max(1),
    );
    println!("arquivo: {path}");
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Este exemplo requer Windows.");
}
