//! Transmissão simulada: reproduz a mídia pré-codificada em tempo real
//! e distribui cada mensagem uma única vez para todos os espectadores.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::time::{Instant, sleep_until};
use vellcast_protocol::{Header, MessageType, encode, flags};

use crate::media::{parse_h264, parse_ogg_opus};

/// Capacidade da fila por espectador, em mensagens (~1 s de vídeo + áudio a 60 fps).
/// Um espectador que fica mais atrasado que isso perde as mensagens mais antigas
/// e volta a sincronizar no próximo keyframe.
pub const QUEUE_CAPACITY: usize = 128;

/// Descrição de um perfil de teste (`<nome>.json` em `VELLCAST_MEDIA_DIR`).
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video: String,
    pub audio: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInit {
    pub codec: String,
    pub coded_width: u32,
    pub coded_height: u32,
    pub framerate: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInit {
    pub codec: &'static str,
    pub sample_rate: u32,
    pub number_of_channels: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub has_audio: bool,
    pub video_bitrate_bps: u64,
}

#[derive(Default)]
pub struct StreamStats {
    pub frames_sent: AtomicU64,
    pub bytes_broadcast: AtomicU64,
}

struct TimelineItem {
    pts_us: u64,
    kind: MessageType,
    keyframe: bool,
    data: Bytes,
}

pub struct TestStream {
    pub info: StreamInfo,
    /// Mensagens `VideoInit` e `AudioInit` enviadas a cada novo espectador.
    pub init_messages: Vec<Bytes>,
    pub sender: broadcast::Sender<Bytes>,
    pub stats: StreamStats,
    timeline: Vec<TimelineItem>,
    loop_duration_us: u64,
}

pub fn now_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

impl TestStream {
    pub fn load(id: u32, dir: &Path, manifest: Manifest) -> Result<Self> {
        let video_path = dir.join(&manifest.video);
        let raw = std::fs::read(&video_path)
            .with_context(|| format!("lendo {}", video_path.display()))?;
        let h264 = parse_h264(&raw).with_context(|| format!("{}", video_path.display()))?;

        let frame_us = 1_000_000 / u64::from(manifest.fps.max(1));
        let mut timeline: Vec<TimelineItem> = h264
            .access_units
            .iter()
            .enumerate()
            .map(|(i, au)| TimelineItem {
                pts_us: i as u64 * frame_us,
                kind: MessageType::VideoFrame,
                keyframe: au.keyframe,
                data: au.data.clone(),
            })
            .collect();
        let video_bytes: usize = h264.access_units.iter().map(|au| au.data.len()).sum();
        let video_duration_us = h264.access_units.len() as u64 * frame_us;

        let mut init_messages = vec![encode(
            &Header::new(MessageType::VideoInit, id),
            &serde_json::to_vec(&VideoInit {
                codec: h264.codec.clone(),
                coded_width: manifest.width,
                coded_height: manifest.height,
                framerate: manifest.fps,
            })?,
        )?];

        let mut loop_duration_us = video_duration_us;
        if let Some(audio) = &manifest.audio {
            let audio_path = dir.join(audio);
            let raw = std::fs::read(&audio_path)
                .with_context(|| format!("lendo {}", audio_path.display()))?;
            let opus = parse_ogg_opus(&raw).with_context(|| format!("{}", audio_path.display()))?;
            init_messages.push(encode(
                &Header::new(MessageType::AudioInit, id),
                &serde_json::to_vec(&AudioInit {
                    codec: "opus",
                    sample_rate: opus.sample_rate,
                    number_of_channels: opus.channels,
                })?,
            )?);
            let mut pts = 0;
            for (packet, dur) in opus.packets {
                // Descarta áudio além da duração do vídeo para manter o loop alinhado.
                if pts >= video_duration_us {
                    break;
                }
                timeline.push(TimelineItem {
                    pts_us: pts,
                    kind: MessageType::AudioFrame,
                    keyframe: false,
                    data: packet,
                });
                pts += dur;
            }
            loop_duration_us = loop_duration_us.max(pts);
        }
        timeline.sort_by_key(|item| item.pts_us);

        let (sender, _) = broadcast::channel(QUEUE_CAPACITY);
        Ok(Self {
            info: StreamInfo {
                id,
                name: manifest.name,
                width: manifest.width,
                height: manifest.height,
                fps: manifest.fps,
                codec: h264.codec,
                has_audio: manifest.audio.is_some(),
                video_bitrate_bps: (video_bytes as u64 * 8 * 1_000_000) / video_duration_us.max(1),
            },
            init_messages,
            sender,
            stats: StreamStats::default(),
            timeline,
            loop_duration_us,
        })
    }

    /// Reproduz a linha do tempo em loop, em tempo real.
    ///
    /// Os PTS continuam crescendo entre loops para que o player
    /// não perceba descontinuidade.
    pub async fn run(self: Arc<Self>) {
        let start = Instant::now();
        let mut sequence: u32 = 0;
        let mut offset = 0;
        loop {
            for item in &self.timeline {
                let pts_us = offset + item.pts_us;
                sleep_until(start + Duration::from_micros(pts_us)).await;
                if self.sender.receiver_count() == 0 {
                    continue;
                }
                let mut header = Header::new(item.kind, self.info.id);
                header.sequence = sequence;
                header.pts_us = pts_us;
                header.sent_at_us = now_us();
                if item.keyframe {
                    header.flags |= flags::KEYFRAME;
                }
                sequence = sequence.wrapping_add(1);
                let Ok(message) = encode(&header, &item.data) else {
                    tracing::warn!(stream = %self.info.name, "quadro excede o limite do protocolo");
                    continue;
                };
                if item.kind == MessageType::VideoFrame {
                    self.stats.frames_sent.fetch_add(1, Ordering::Relaxed);
                }
                self.stats
                    .bytes_broadcast
                    .fetch_add(message.len() as u64, Ordering::Relaxed);
                // Erro só ocorre sem receptores; nesse caso o quadro é descartado.
                let _ = self.sender.send(message);
            }
            offset += self.loop_duration_us;
        }
    }
}

/// Carrega todos os manifests `*.json` do diretório de mídia.
pub fn load_all(dir: &Path) -> Result<Vec<Arc<TestStream>>> {
    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("lendo {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            let manifest: Manifest = serde_json::from_slice(&std::fs::read(&path)?)
                .with_context(|| format!("manifest inválido: {}", path.display()))?;
            manifests.push(manifest);
        }
    }
    manifests.sort_by(|a, b| a.name.cmp(&b.name));
    manifests
        .into_iter()
        .enumerate()
        .map(|(i, m)| TestStream::load(i as u32 + 1, dir, m).map(Arc::new))
        .collect()
}
