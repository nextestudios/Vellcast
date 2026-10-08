//! Publica a mídia sintética da ETAPA 0 em um relay, em tempo real e em loop.
//!
//! ```text
//! VELLCAST_PUBLISHER_TOKEN=<token> cargo run --release -p vellcast-transport \
//!   --example publish_fixture -- ws://127.0.0.1:8788/ws 1080p60 [segundos]
//! ```
//! O token vem de `POST /api/rooms` (campo `publisherToken`); nunca da linha de comando,
//! para não aparecer no histórico do shell nem na lista de processos.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::time::{Instant, sleep_until};
use vellcast_media_fixtures::MediaKind;
use vellcast_transport::{Publisher, PublisherConfig, PublisherEvent};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let url = args
        .next()
        .unwrap_or_else(|| "ws://127.0.0.1:8788/ws".into());
    let profile = args.next().unwrap_or_else(|| "1080p60".into());
    let seconds: Option<u64> = args.next().and_then(|s| s.parse().ok());
    let token =
        std::env::var("VELLCAST_PUBLISHER_TOKEN").context("defina VELLCAST_PUBLISHER_TOKEN")?;
    let dir = PathBuf::from(
        std::env::var("VELLCAST_MEDIA_DIR").unwrap_or_else(|_| "tests/fixtures/media".into()),
    );
    let fixture = vellcast_media_fixtures::load(&dir, &profile)?;
    tracing::info!(
        profile,
        bitrate_kbps = fixture.video_bitrate_bps / 1000,
        "mídia carregada"
    );

    let (publisher, mut events) = Publisher::connect(PublisherConfig::new(url, token));
    publisher.set_video_init(&fixture.video_init);
    if let Some(audio) = &fixture.audio_init {
        publisher.set_audio_init(audio);
    }
    tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            match ev {
                // A mídia pré-codificada já tem keyframe a cada 1 s.
                PublisherEvent::KeyframeRequested => tracing::debug!("keyframe solicitado"),
                other => tracing::info!(?other, "evento"),
            }
        }
    });

    let start = Instant::now();
    let deadline = seconds.map(|s| start + Duration::from_secs(s));
    let mut offset = 0;
    'outer: loop {
        for item in &fixture.timeline {
            let pts = offset + item.pts_us;
            let at = start + Duration::from_micros(pts);
            if deadline.is_some_and(|d| at >= d) {
                break 'outer;
            }
            sleep_until(at).await;
            match item.kind {
                MediaKind::Video => publisher.send_video(&item.data, pts, item.keyframe),
                MediaKind::Audio => publisher.send_audio(&item.data, pts),
            };
        }
        offset += fixture.loop_duration_us;
    }
    let stats = publisher.stats();
    tracing::info!(?stats, "encerrando");
    publisher.finish(Duration::from_secs(5)).await;
    Ok(())
}
