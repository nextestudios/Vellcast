//! Avaliação QUIC (Quinn) × WebSocket/TCP entre transmissor e relay (S-005, E6-T05).
//!
//! Um transmissor sintético (perfil 1080p60, keyframe a cada 2 s) envia quadros a
//! um receptor através de um proxy de perturbação no mesmo processo:
//!
//! - **WebSocket/TCP** (o transporte atual): proxy TCP que atrasa os dados e
//!   modela perdas como retransmissões. TCP entrega em ordem, então um trecho
//!   "perdido" atrasa tudo o que vem depois (bloqueio de cabeça de fila). A perda
//!   real de segmentos TCP não pode ser simulada no espaço do usuário; o modelo
//!   aplica, a cada trecho, a probabilidade de conter um pacote perdido e uma
//!   penalidade de retransmissão rápida (1 RTT + 10 ms); no apagão, RTO de 200 ms.
//! - **QUIC, um stream unidirecional por quadro** (prioridade FIFO), com Cubic
//!   (padrão do Quinn) e com BBR: proxy UDP que descarta pacotes com a
//!   probabilidade configurada e os atrasa sem reordenar. A perda de um pacote só
//!   atrasa o quadro a que ele pertence.
//!
//! Limitação importante: o modelo de TCP não reduz a janela de congestionamento
//! após perdas (só atrasa os dados), então os números de TCP com perda são
//! otimistas. Um TCP real com Cubic/Reno sofre o mesmo limite de vazão que o
//! QUIC-Cubic (fórmula de Mathis).
//!
//! ```text
//! cargo run --release -p vellcast-perf --bin quic_eval -- [segundos por cenário=10]
//! ```
//!
//! Saída: JSON por cenário e transporte com latência por quadro (envio → quadro
//! completo no receptor), quadros entregues, vazão, CPU do processo, RSS máximo e
//! tempo de recuperação após um apagão de 300 ms.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::{BufMut, BytesMut};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio_tungstenite::tungstenite::Message;
use vellcast_protocol::{HEADER_LEN, Header, MessageType, encode, flags};

const FPS: u32 = 60;
const KBPS: u64 = 8000;
const BLACKOUT_AT: Duration = Duration::from_secs(4);
const BLACKOUT_LEN: Duration = Duration::from_millis(300);
const MSS: usize = 1460;

/// Perturbação aplicada pelo proxy.
#[derive(Clone, Copy)]
struct Impairment {
    name: &'static str,
    /// Atraso de ida (ms); o RTT é o dobro.
    delay_ms: u64,
    jitter_ms: u64,
    loss: f64,
    blackout: bool,
}

/// PRNG determinístico (xorshift64*): resultados reproduzíveis.
struct Rng(u64);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

fn process_cpu_secs() -> f64 {
    // SAFETY: `getrusage` preenche a struct zerada passada por ponteiro válido.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: ponteiro para a struct local.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(usage.ru_utime) + tv(usage.ru_stime)
}

fn max_rss_mb() -> f64 {
    // SAFETY: idem.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: ponteiro para a struct local.
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    // macOS informa em bytes; Linux em KiB.
    if cfg!(target_os = "macos") {
        usage.ru_maxrss as f64 / 1e6
    } else {
        usage.ru_maxrss as f64 / 1e3
    }
}

/// Quadros sintéticos: tamanho e se é keyframe.
fn frame_sizes(seconds: u64) -> Vec<(usize, bool)> {
    let gop = (FPS * 2) as usize;
    let avg = (KBPS * 1000 / 8 / u64::from(FPS)) as usize;
    let p = avg * gop / (gop + 7);
    (0..(seconds as usize * FPS as usize))
        .map(|i| {
            if i % gop == 0 {
                (p * 8, true)
            } else {
                (p, false)
            }
        })
        .collect()
}

fn frame_bytes(seq: u32, size: usize, keyframe: bool) -> bytes::Bytes {
    let mut h = Header::new(MessageType::VideoFrame, 0);
    h.sequence = seq;
    h.sent_at_us = now_us();
    if keyframe {
        h.flags |= flags::KEYFRAME;
    }
    let mut buf = BytesMut::with_capacity(HEADER_LEN + size);
    buf.put_slice(&encode(&h, &[]).unwrap());
    buf.put_bytes(0x42, size);
    buf.freeze()
}

/// Registro de chegada: (instante relativo ao início, latência em µs, instante de envio relativo).
type Arrivals = Arc<Mutex<Vec<(Duration, u64)>>>;

fn in_blackout(imp: &Impairment, elapsed: Duration) -> bool {
    imp.blackout && elapsed >= BLACKOUT_AT && elapsed < BLACKOUT_AT + BLACKOUT_LEN
}

// ---------------------------------------------------------------------------
// TCP/WebSocket

/// Encaminha `from` → `to` aplicando atraso, perdas (como retransmissões) e
/// apagão, sempre preservando a ordem (como o TCP).
async fn tcp_pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    imp: Impairment,
    start: Instant,
    seed: u64,
    impaired: bool,
) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let writer = tokio::spawn(async move {
        while let Some((at, data)) = rx.recv().await {
            tokio::time::sleep_until(at.into()).await;
            if to.write_all(&data).await.is_err() {
                break;
            }
        }
    });
    let mut rng = Rng(seed);
    let mut last = Instant::now();
    let mut buf = vec![0u8; 16 * 1024];
    let rtt = Duration::from_millis(imp.delay_ms * 2);
    loop {
        let n = match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let now = Instant::now();
        let mut at = now + Duration::from_millis(imp.delay_ms);
        if impaired {
            at += Duration::from_micros((rng.next_f64() * imp.jitter_ms as f64 * 1000.0) as u64);
            let packets = n.div_ceil(MSS) as i32;
            let p_chunk = 1.0 - (1.0 - imp.loss).powi(packets);
            if rng.next_f64() < p_chunk {
                // Retransmissão rápida: ~1 RTT extra.
                at += rtt + Duration::from_millis(10);
            }
            let elapsed = now - start;
            if in_blackout(&imp, elapsed) {
                // Tudo enviado durante o apagão chega após o RTO (200 ms) contado do fim.
                at = at.max(start + BLACKOUT_AT + BLACKOUT_LEN + Duration::from_millis(200));
            }
        }
        // Entrega em ordem: nada passa à frente do que ficou retido.
        at = at.max(last);
        last = at;
        if tx.send((at, buf[..n].to_vec())).is_err() {
            break;
        }
    }
    drop(tx);
    let _ = writer.await;
}

async fn run_websocket(imp: Impairment, seconds: u64) -> Value {
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = server.local_addr().unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy.local_addr().unwrap();
    let start = Instant::now();
    let arrivals: Arrivals = Arc::default();

    // Receptor (papel do relay).
    let rec = Arc::clone(&arrivals);
    let receiver = tokio::spawn(async move {
        let (sock, _) = server.accept().await.unwrap();
        sock.set_nodelay(true).unwrap();
        let mut ws = tokio_tungstenite::accept_async(sock).await.unwrap();
        while let Some(Ok(msg)) = ws.next().await {
            if let Message::Binary(data) = msg
                && let Ok(h) = Header::read(&data)
            {
                rec.lock()
                    .unwrap()
                    .push((start.elapsed(), now_us().saturating_sub(h.sent_at_us)));
            }
        }
    });
    // Proxy: cliente → servidor com perturbação; volta só com atraso.
    let proxy_task = tokio::spawn(async move {
        let (client, _) = proxy.accept().await.unwrap();
        let upstream = TcpStream::connect(server_addr).await.unwrap();
        client.set_nodelay(true).unwrap();
        upstream.set_nodelay(true).unwrap();
        let (cr, cw) = client.into_split();
        let (ur, uw) = upstream.into_split();
        let a = tokio::spawn(tcp_pipe(cr, uw, imp, start, 7, true));
        let b = tokio::spawn(tcp_pipe(ur, cw, imp, start, 11, false));
        let _ = tokio::join!(a, b);
    });

    let sock = TcpStream::connect(proxy_addr).await.unwrap();
    sock.set_nodelay(true).unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async(format!("ws://{proxy_addr}/"), sock)
        .await
        .unwrap();
    let cpu0 = process_cpu_secs();
    let frames = frame_sizes(seconds);
    let interval = Duration::from_secs(1) / FPS;
    let mut sent_bytes = 0u64;
    for (i, (size, key)) in frames.iter().enumerate() {
        tokio::time::sleep_until((start + interval * i as u32).into()).await;
        let data = frame_bytes(i as u32, *size, *key);
        sent_bytes += data.len() as u64;
        if ws.send(Message::Binary(data)).await.is_err() {
            break;
        }
    }
    // Espera a fila drenar (o apagão e as retransmissões atrasam o fim).
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let cpu = process_cpu_secs() - cpu0;
    let _ = ws.close(None).await;
    proxy_task.abort();
    receiver.abort();
    summarize(
        "websocket-tcp",
        &imp,
        &arrivals,
        frames.len(),
        sent_bytes,
        cpu,
        start.elapsed(),
    )
}

// ---------------------------------------------------------------------------
// QUIC

/// Encaminha datagramas com atraso preservando a ordem (como o pipe TCP):
/// o jitter varia o atraso sem reordenar pacotes.
fn ordered_sender(
    sock: Arc<UdpSocket>,
) -> tokio::sync::mpsc::UnboundedSender<(Instant, Vec<u8>, SocketAddr)> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Instant, Vec<u8>, SocketAddr)>();
    tokio::spawn(async move {
        while let Some((at, data, to)) = rx.recv().await {
            tokio::time::sleep_until(at.into()).await;
            let _ = sock.send_to(&data, to).await;
        }
    });
    tx
}

/// Proxy UDP com perda, atraso e apagão nos dois sentidos.
async fn udp_proxy(imp: Impairment, server: SocketAddr, start: Instant) -> SocketAddr {
    let front = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let back = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let addr = front.local_addr().unwrap();
    let client: Arc<Mutex<Option<SocketAddr>>> = Arc::default();
    let up = {
        let (front, client) = (Arc::clone(&front), Arc::clone(&client));
        let out = ordered_sender(Arc::clone(&back));
        async move {
            let mut rng = Rng(13);
            let mut last = Instant::now();
            let mut buf = vec![0u8; 65_536];
            loop {
                let Ok((n, from)) = front.recv_from(&mut buf).await else {
                    break;
                };
                *client.lock().unwrap() = Some(from);
                if in_blackout(&imp, start.elapsed()) || rng.next_f64() < imp.loss {
                    continue;
                }
                let delay = imp.delay_ms as f64 + rng.next_f64() * imp.jitter_ms as f64;
                let at =
                    (Instant::now() + Duration::from_micros((delay * 1000.0) as u64)).max(last);
                last = at;
                if out.send((at, buf[..n].to_vec(), server)).is_err() {
                    break;
                }
            }
        }
    };
    let down = {
        let (back, client) = (Arc::clone(&back), Arc::clone(&client));
        let out = ordered_sender(Arc::clone(&front));
        async move {
            let mut rng = Rng(17);
            let mut buf = vec![0u8; 65_536];
            loop {
                let Ok((n, _)) = back.recv_from(&mut buf).await else {
                    break;
                };
                let Some(to) = *client.lock().unwrap() else {
                    continue;
                };
                if in_blackout(&imp, start.elapsed()) || rng.next_f64() < imp.loss {
                    continue;
                }
                let at = Instant::now() + Duration::from_millis(imp.delay_ms);
                if out.send((at, buf[..n].to_vec(), to)).is_err() {
                    break;
                }
            }
        }
    };
    tokio::spawn(up);
    tokio::spawn(down);
    addr
}

async fn run_quic(imp: Impairment, seconds: u64, bbr: bool) -> Value {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = certified.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
    let mut server_config =
        quinn::ServerConfig::with_single_cert(vec![cert_der.clone()], key.into()).unwrap();
    let mut transport = quinn::TransportConfig::default();
    transport.max_concurrent_uni_streams(4096u32.into());
    transport.stream_receive_window((8u32 * 1024 * 1024).into());
    transport.receive_window((32u32 * 1024 * 1024).into());
    if bbr {
        // BBR estima banda e RTT em vez de reagir a cada perda (Cubic é o padrão).
        transport.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    }
    let transport = Arc::new(transport);
    server_config.transport_config(Arc::clone(&transport));
    let server = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server.local_addr().unwrap();
    let start = Instant::now();
    let proxy_addr = udp_proxy(imp, server_addr, start).await;
    let arrivals: Arrivals = Arc::default();

    let rec = Arc::clone(&arrivals);
    let accepted = Arc::new(AtomicU64::new(0));
    let read_errors = Arc::new(AtomicU64::new(0));
    let (acc, errs) = (Arc::clone(&accepted), Arc::clone(&read_errors));
    let receiver = tokio::spawn(async move {
        let conn = server.accept().await.unwrap().await.unwrap();
        while let Ok(mut stream) = conn.accept_uni().await {
            acc.fetch_add(1, Ordering::Relaxed);
            let (rec, errs) = (Arc::clone(&rec), Arc::clone(&errs));
            tokio::spawn(async move {
                match stream.read_to_end(4 * 1024 * 1024).await {
                    Ok(data) => {
                        if let Ok(h) = Header::read(&data) {
                            rec.lock()
                                .unwrap()
                                .push((start.elapsed(), now_us().saturating_sub(h.sent_at_us)));
                        }
                    }
                    Err(e) => {
                        if errs.fetch_add(1, Ordering::Relaxed) == 0 {
                            eprintln!("quic: erro de leitura: {e}");
                        }
                    }
                }
            });
        }
    });

    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert_der).unwrap();
    let mut client_config = quinn::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
    client_config.transport_config(transport);
    let mut client = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
    client.set_default_client_config(client_config);
    let conn = client
        .connect(proxy_addr, "localhost")
        .unwrap()
        .await
        .unwrap();

    let cpu0 = process_cpu_secs();
    let frames = frame_sizes(seconds);
    let interval = Duration::from_secs(1) / FPS;
    let mut sent_bytes = 0u64;
    let pending = Arc::new(AtomicU64::new(0));
    for (i, (size, key)) in frames.iter().enumerate() {
        tokio::time::sleep_until((start + interval * i as u32).into()).await;
        let data = frame_bytes(i as u32, *size, *key);
        sent_bytes += data.len() as u64;
        let Ok(mut stream) = conn.open_uni().await else {
            break;
        };
        // FIFO: o quadro mais antigo tem prioridade (sem isso o Quinn reparte a
        // banda entre todos os streams abertos e nenhum termina a tempo).
        let _ = stream.set_priority(-(i as i32));
        let pending = Arc::clone(&pending);
        pending.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            let _ = stream.write_all(&data).await;
            let _ = stream.finish();
            let _ = stream.stopped().await;
            pending.fetch_sub(1, Ordering::Relaxed);
        });
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let cpu = process_cpu_secs() - cpu0;
    eprintln!(
        "quic: streams aceitos {}, erros {}, pendentes no envio {}",
        accepted.load(Ordering::Relaxed),
        read_errors.load(Ordering::Relaxed),
        pending.load(Ordering::Relaxed)
    );
    conn.close(0u32.into(), b"fim");
    receiver.abort();
    let name = if bbr { "quic-bbr" } else { "quic-cubic" };
    summarize(
        name,
        &imp,
        &arrivals,
        frames.len(),
        sent_bytes,
        cpu,
        start.elapsed(),
    )
}

// ---------------------------------------------------------------------------

fn pct(sorted: &[u64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[(((p / 100.0) * sorted.len() as f64) as usize).min(sorted.len() - 1)] as f64 / 1000.0
}

fn summarize(
    transport: &str,
    imp: &Impairment,
    arrivals: &Arrivals,
    sent: usize,
    sent_bytes: u64,
    cpu: f64,
    wall: Duration,
) -> Value {
    let arr = arrivals.lock().unwrap().clone();
    let mut lat: Vec<u64> = arr.iter().map(|(_, l)| *l).collect();
    lat.sort_unstable();
    // Recuperação: após o fim do apagão, primeiro quadro que chega com latência
    // de volta ao normal (≤ RTT de ida + 30 ms).
    let recovery_ms = imp.blackout.then(|| {
        let end = BLACKOUT_AT + BLACKOUT_LEN;
        let threshold = (imp.delay_ms + imp.jitter_ms + 30) * 1000;
        arr.iter()
            .filter(|(at, l)| *at >= end && *l <= threshold)
            .map(|(at, _)| (*at - end).as_secs_f64() * 1000.0)
            .next()
            .unwrap_or(f64::NAN)
    });
    let mut late: VecDeque<u64> = lat.iter().copied().filter(|l| *l > 200_000).collect();
    json!({
        "transport": transport,
        "scenario": imp.name,
        "delayMs": imp.delay_ms,
        "jitterMs": imp.jitter_ms,
        "loss": imp.loss,
        "framesSent": sent,
        "framesDelivered": arr.len(),
        "framesOver200ms": late.drain(..).count(),
        "throughputMbps": sent_bytes as f64 * 8.0 / 1e6 / wall.as_secs_f64(),
        "latencyMs": { "p50": pct(&lat, 50.0), "p95": pct(&lat, 95.0), "p99": pct(&lat, 99.0), "max": pct(&lat, 100.0) },
        "recoveryAfterBlackoutMs": recovery_ms,
        "processCpuPercent": cpu / wall.as_secs_f64() * 100.0,
        "maxRssMb": max_rss_mb(),
    })
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    let scenarios = [
        Impairment {
            name: "limpo",
            delay_ms: 20,
            jitter_ms: 2,
            loss: 0.0,
            blackout: false,
        },
        Impairment {
            name: "perda-1%",
            delay_ms: 20,
            jitter_ms: 2,
            loss: 0.01,
            blackout: false,
        },
        Impairment {
            name: "perda-3%",
            delay_ms: 40,
            jitter_ms: 5,
            loss: 0.03,
            blackout: false,
        },
        Impairment {
            name: "apagao-300ms",
            delay_ms: 20,
            jitter_ms: 2,
            loss: 0.0,
            blackout: true,
        },
    ];
    let mut results = Vec::new();
    for imp in scenarios {
        results.push(run_websocket(imp, seconds).await);
        results.push(run_quic(imp, seconds, false).await);
        results.push(run_quic(imp, seconds, true).await);
        eprintln!("cenário {} concluído", imp.name);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&Value::Array(results)).unwrap()
    );
}
