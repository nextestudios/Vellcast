//! Benchmark reproduzível da distribuição (fan-out) do relay.
//!
//! O relay roda em um runtime Tokio próprio dentro deste processo; transmissor
//! e espectadores (clientes WebSocket reais) rodam em outro. Um alocador
//! contador soma apenas as alocações feitas nas threads do relay, separando o
//! custo do servidor do custo dos clientes.
//!
//! ```text
//! cargo run --release -p vellcast-relay --example fanout_bench -- \
//!     [espectadores=20] [segundos=20] [fps=60] [kbps=8000] [batch=on|off] [threads do relay=2]
//! ```
//!
//! Saída: JSON com mensagens entregues, alocações e bytes alocados por mensagem
//! entregue (somente threads do relay), CPU do processo e FPS/latência vistos
//! pelos espectadores. Mesma máquina e mesmo relógio: a latência é relay +
//! escalonamento, sem rede real.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::{BufMut, BytesMut};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use vellcast_protocol::messages::{AudioInit, AuthRequest, ConnectRole, VideoInit, encode_json};
use vellcast_protocol::{HEADER_LEN, Header, MessageType, encode, flags};
use vellcast_relay::config::Config;
use vellcast_relay::{Relay, now_us};

// ---------------------------------------------------------------------------
// Alocador contador (somente threads marcadas como relay)

struct Counting;

static RELAY_ALLOCS: AtomicU64 = AtomicU64::new(0);
static RELAY_BYTES: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static IN_RELAY: Cell<bool> = const { Cell::new(false) };
}

fn count(size: usize) {
    if IN_RELAY.try_with(Cell::get).unwrap_or(false) {
        RELAY_ALLOCS.fetch_add(1, Ordering::Relaxed);
        RELAY_BYTES.fetch_add(size as u64, Ordering::Relaxed);
    }
}

// SAFETY: delega tudo ao alocador do sistema; só soma contadores atômicos.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: mesmo contrato de `GlobalAlloc::alloc`.
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: mesmo contrato de `GlobalAlloc::alloc_zeroed`.
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        // SAFETY: mesmo contrato de `GlobalAlloc::realloc`.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: mesmo contrato de `GlobalAlloc::dealloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// ---------------------------------------------------------------------------

/// Tempo de CPU (usuário + sistema) do processo inteiro, em segundos.
#[cfg(unix)]
fn process_cpu_secs() -> f64 {
    // SAFETY: `getrusage` preenche a struct zerada passada por ponteiro.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: ponteiro válido para a struct local.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(usage.ru_utime) + tv(usage.ru_stime)
}

/// Cliente HTTP/1.1 mínimo para a API do relay.
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Value,
) -> Value {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = body.to_string();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: bench\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(t) = bearer {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    text.split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null)
}

fn auth(token: &str, role: ConnectRole) -> Message {
    Message::Binary(
        encode_json(
            MessageType::Auth,
            0,
            &AuthRequest {
                token: token.into(),
                role,
            },
        )
        .unwrap(),
    )
}

#[derive(Default)]
struct ViewerResult {
    frames: u64,
    latencies_us: Vec<u64>,
}

async fn viewer(
    addr: SocketAddr,
    token: String,
    until: Instant,
    measure_from: Instant,
) -> ViewerResult {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    ws.send(auth(&token, ConnectRole::Viewer)).await.unwrap();
    let mut r = ViewerResult::default();
    let mut ping = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            msg = ws.next() => {
                let Some(Ok(Message::Binary(data))) = msg else { break };
                if data.len() < HEADER_LEN { continue; }
                let Ok(h) = Header::read(&data) else { continue };
                let now = Instant::now();
                if h.kind == MessageType::VideoFrame && now >= measure_from {
                    r.frames += 1;
                    r.latencies_us.push(now_us().saturating_sub(h.sent_at_us));
                }
                if now >= until { break; }
            }
            _ = ping.tick() => {
                let msg = encode(&Header::new(MessageType::Ping, 0), &[0u8; 8]).unwrap();
                if ws.send(Message::Binary(msg)).await.is_err() { break; }
            }
        }
    }
    r
}

async fn publisher(addr: SocketAddr, token: String, fps: u32, kbps: u32, until: Instant) {
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
        .await
        .unwrap();
    ws.send(auth(&token, ConnectRole::Publisher)).await.unwrap();
    let video_init = VideoInit {
        codec: "avc1.64002A".into(),
        coded_width: 1920,
        coded_height: 1080,
        framerate: fps,
    };
    let audio_init = AudioInit {
        codec: "opus".into(),
        sample_rate: 48_000,
        number_of_channels: 2,
    };
    ws.send(Message::Binary(
        encode_json(MessageType::VideoInit, 0, &video_init).unwrap(),
    ))
    .await
    .unwrap();
    ws.send(Message::Binary(
        encode_json(MessageType::AudioInit, 0, &audio_init).unwrap(),
    ))
    .await
    .unwrap();

    // Perfil próximo de um encoder real: keyframe a cada 2 s com ~8x o tamanho médio.
    let gop = fps * 2;
    let avg = (u64::from(kbps) * 1000 / 8 / u64::from(fps)) as usize;
    let p_size = avg * gop as usize / (gop as usize + 7);
    let i_size = p_size * 8;
    let frame_interval = Duration::from_secs(1) / fps;
    let start = Instant::now();
    let mut seq = 0u32;
    let mut next_video = start;
    let mut next_audio = start;
    let audio_payload = vec![0x5Au8; 300];
    let mut frame = 0u32;
    while Instant::now() < until {
        let now = Instant::now();
        if now >= next_video {
            let keyframe = frame.is_multiple_of(gop);
            let size = if keyframe { i_size } else { p_size };
            let mut h = Header::new(MessageType::VideoFrame, 0);
            h.sequence = seq;
            h.pts_us = start.elapsed().as_micros() as u64;
            h.sent_at_us = now_us();
            if keyframe {
                h.flags |= flags::KEYFRAME;
            }
            let mut buf = BytesMut::with_capacity(HEADER_LEN + size);
            buf.put_slice(&encode(&h, &[]).unwrap());
            buf.put_bytes(0x42, size);
            ws.send(Message::Binary(buf.freeze())).await.unwrap();
            seq = seq.wrapping_add(1);
            frame += 1;
            next_video += frame_interval;
        }
        if now >= next_audio {
            let mut h = Header::new(MessageType::AudioFrame, 0);
            h.sequence = seq;
            h.sent_at_us = now_us();
            ws.send(Message::Binary(encode(&h, &audio_payload).unwrap()))
                .await
                .unwrap();
            seq = seq.wrapping_add(1);
            next_audio += Duration::from_millis(20);
        }
        let wake = next_video.min(next_audio);
        tokio::time::sleep_until(wake.into()).await;
    }
    let _ = ws.close(None).await;
}

fn pct(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((p / 100.0) * sorted.len() as f64) as usize % sorted.len()] as f64 / 1000.0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize, d: &str| args.get(i).cloned().unwrap_or_else(|| d.to_string());
    let viewers: usize = arg(0, "20").parse().unwrap();
    let seconds: u64 = arg(1, "20").parse().unwrap();
    let fps: u32 = arg(2, "60").parse().unwrap();
    let kbps: u32 = arg(3, "8000").parse().unwrap();
    let batch = arg(4, "on") != "off";
    let relay_threads: usize = arg(5, "2").parse().unwrap();

    // Runtime do relay: threads marcadas para o alocador contador.
    let relay_rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(relay_threads)
        .thread_name("relay")
        .on_thread_start(|| IN_RELAY.with(|f| f.set(true)))
        .enable_all()
        .build()
        .unwrap();
    let mut config = Config::with_secret(b"bench-secret-bench-secret-bench-s".to_vec());
    config.api_burst = 10_000.0;
    config.api_per_sec = 10_000.0;
    config.ws_burst = 10_000.0;
    config.ws_per_sec = 10_000.0;
    config.max_viewers_per_room = viewers.max(1) + 1;
    config.batch_writes = batch;
    let relay = Relay::new(config);
    let listener = relay_rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap();
    relay_rt.spawn(vellcast_relay::serve(Arc::clone(&relay), listener));

    let clients = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("clients")
        .enable_all()
        .build()
        .unwrap();
    let report = clients.block_on(async {
        let created = http(
            addr,
            "POST",
            "/api/rooms",
            None,
            json!({ "name": "bench", "maxViewers": viewers }),
        )
        .await;
        let room = created["room"]["id"].as_str().unwrap().to_string();
        let owner = created["ownerToken"].as_str().unwrap().to_string();
        let pub_token = created["publisherToken"].as_str().unwrap().to_string();
        let invite = http(
            addr,
            "POST",
            &format!("/api/rooms/{room}/invites"),
            Some(&owner),
            json!({ "maxUses": viewers }),
        )
        .await;
        let mut tokens = Vec::new();
        for _ in 0..viewers {
            let joined = http(
                addr,
                "POST",
                &format!("/api/rooms/{room}/join"),
                None,
                json!({ "invite": invite["code"] }),
            )
            .await;
            tokens.push(joined["viewerToken"].as_str().unwrap().to_string());
        }

        let warmup = Duration::from_secs(2);
        let t0 = Instant::now();
        let measure_from = t0 + warmup;
        let until = measure_from + Duration::from_secs(seconds);
        let pub_task = tokio::spawn(publisher(
            addr,
            pub_token,
            fps,
            kbps,
            until + Duration::from_millis(500),
        ));
        let mut tasks = Vec::new();
        for t in tokens {
            tasks.push(tokio::spawn(viewer(addr, t, until, measure_from)));
        }

        tokio::time::sleep_until(measure_from.into()).await;
        let a0 = RELAY_ALLOCS.load(Ordering::Relaxed);
        let b0 = RELAY_BYTES.load(Ordering::Relaxed);
        let cpu0 = process_cpu_secs();
        let out0 = relay.metrics.messages_out.load(Ordering::Relaxed);
        let bytes_out0 = relay.metrics.bytes_out.load(Ordering::Relaxed);
        let wall0 = Instant::now();

        let mut results = Vec::new();
        for t in tasks {
            results.push(t.await.unwrap());
        }
        let wall = wall0.elapsed().as_secs_f64();
        let a1 = RELAY_ALLOCS.load(Ordering::Relaxed);
        let b1 = RELAY_BYTES.load(Ordering::Relaxed);
        let cpu1 = process_cpu_secs();
        let out1 = relay.metrics.messages_out.load(Ordering::Relaxed);
        let bytes_out1 = relay.metrics.bytes_out.load(Ordering::Relaxed);
        let _ = pub_task.await;

        let delivered = (out1 - out0).max(1);
        let mut lat: Vec<u64> = results
            .iter()
            .flat_map(|r| r.latencies_us.iter().copied())
            .collect();
        lat.sort_unstable();
        let fps_per_viewer: Vec<f64> = results
            .iter()
            .map(|r| r.frames as f64 / seconds as f64)
            .collect();
        json!({
            "viewers": viewers,
            "seconds": seconds,
            "fps": fps,
            "kbps": kbps,
            "batchWrites": batch,
            "relayThreads": relay_threads,
            "messagesDelivered": out1 - out0,
            "mbpsOut": ((bytes_out1 - bytes_out0) as f64 * 8.0 / 1e6 / wall),
            "relayAllocsPerMessage": (a1 - a0) as f64 / delivered as f64,
            "relayAllocBytesPerMessage": (b1 - b0) as f64 / delivered as f64,
            "relayAllocsPerSec": (a1 - a0) as f64 / wall,
            "processCpuPercent": (cpu1 - cpu0) / wall * 100.0,
            "viewerFps": {
                "min": fps_per_viewer.iter().copied().fold(f64::INFINITY, f64::min),
                "max": fps_per_viewer.iter().copied().fold(0.0, f64::max),
            },
            "latencyMs": { "p50": pct(&lat, 50.0), "p95": pct(&lat, 95.0), "p99": pct(&lat, 99.0) },
            "laggedMessages": relay.metrics.lagged_messages.load(Ordering::Relaxed),
            "staleFramesDropped": relay.metrics.stale_frames_dropped.load(Ordering::Relaxed),
        })
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

/// Fora de Unix (ex.: Windows) a medição de CPU do processo não está disponível.
#[cfg(not(unix))]
fn process_cpu_secs() -> f64 {
    0.0
}
