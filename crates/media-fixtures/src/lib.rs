//! Mídia sintética de teste gerada por `scripts/generate-test-media.sh`.
//!
//! Usada pelo servidor da ETAPA 0, pelo publicador de exemplo do
//! `vellcast-transport` e pelos benchmarks. Não faz parte do produto.

mod parse;

use std::path::Path;

use anyhow::{Context, Result};
use bytes::Bytes;
use serde::Deserialize;
use vellcast_protocol::messages::{AudioInit, VideoInit};

pub use parse::{
    AccessUnit, H264Stream, OpusStream, opus_packet_duration_us, parse_h264, parse_ogg_opus,
    split_nal_units,
};

/// Descrição de um perfil de teste (`<nome>.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub video: String,
    pub audio: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
}

/// Um quadro de vídeo ou pacote de áudio com seu PTS dentro do loop.
#[derive(Debug, Clone)]
pub struct TimelineItem {
    pub pts_us: u64,
    pub kind: MediaKind,
    pub keyframe: bool,
    pub data: Bytes,
}

#[derive(Debug)]
pub struct Fixture {
    pub manifest: Manifest,
    pub video_init: VideoInit,
    pub audio_init: Option<AudioInit>,
    /// Itens ordenados por PTS.
    pub timeline: Vec<TimelineItem>,
    pub loop_duration_us: u64,
    pub video_bitrate_bps: u64,
}

/// Lista os manifests `*.json` de um diretório, ordenados por nome.
pub fn list(dir: &Path) -> Result<Vec<Manifest>> {
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
    Ok(manifests)
}

/// Carrega um perfil pelo nome (ex.: `1080p60`).
pub fn load(dir: &Path, name: &str) -> Result<Fixture> {
    let manifest = list(dir)?
        .into_iter()
        .find(|m| m.name == name)
        .with_context(|| format!("perfil {name} não encontrado em {}", dir.display()))?;
    load_manifest(dir, manifest)
}

pub fn load_manifest(dir: &Path, manifest: Manifest) -> Result<Fixture> {
    let video_path = dir.join(&manifest.video);
    let raw =
        std::fs::read(&video_path).with_context(|| format!("lendo {}", video_path.display()))?;
    let h264 = parse_h264(&raw).with_context(|| video_path.display().to_string())?;

    let frame_us = 1_000_000 / u64::from(manifest.fps.max(1));
    let mut timeline: Vec<TimelineItem> = h264
        .access_units
        .iter()
        .enumerate()
        .map(|(i, au)| TimelineItem {
            pts_us: i as u64 * frame_us,
            kind: MediaKind::Video,
            keyframe: au.keyframe,
            data: au.data.clone(),
        })
        .collect();
    let video_bytes: usize = h264.access_units.iter().map(|au| au.data.len()).sum();
    let video_duration_us = h264.access_units.len() as u64 * frame_us;
    let mut loop_duration_us = video_duration_us;

    let mut audio_init = None;
    if let Some(audio) = &manifest.audio {
        let audio_path = dir.join(audio);
        let raw = std::fs::read(&audio_path)
            .with_context(|| format!("lendo {}", audio_path.display()))?;
        let opus = parse_ogg_opus(&raw).with_context(|| audio_path.display().to_string())?;
        audio_init = Some(AudioInit {
            codec: "opus".into(),
            sample_rate: opus.sample_rate,
            number_of_channels: opus.channels,
        });
        let mut pts = 0;
        for (packet, dur) in opus.packets {
            // Descarta áudio além da duração do vídeo para manter o loop alinhado.
            if pts >= video_duration_us {
                break;
            }
            timeline.push(TimelineItem {
                pts_us: pts,
                kind: MediaKind::Audio,
                keyframe: false,
                data: packet,
            });
            pts += dur;
        }
        loop_duration_us = loop_duration_us.max(pts);
    }
    timeline.sort_by_key(|item| item.pts_us);

    Ok(Fixture {
        video_init: VideoInit {
            codec: h264.codec,
            coded_width: manifest.width,
            coded_height: manifest.height,
            framerate: manifest.fps,
        },
        audio_init,
        timeline,
        loop_duration_us,
        video_bitrate_bps: (video_bytes as u64 * 8 * 1_000_000) / video_duration_us.max(1),
        manifest,
    })
}
