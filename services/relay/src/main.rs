//! Binário do relay. Configuração por variáveis de ambiente (`docs/DEPLOYMENT.md`).

use anyhow::{Context, Result};
use vellcast_relay::{Relay, config::Config, serve_with_shutdown};

#[tokio::main]
async fn main() -> Result<()> {
    init_logs();
    let config = Config::from_env()?;
    let bind = config.bind;
    tracing::info!(
        %bind,
        max_rooms = config.max_rooms,
        max_viewers_per_room = config.max_viewers_per_room,
        max_publisher_kbps = config.max_publisher_kbps,
        create_room_key = config.create_room_key.is_some(),
        trust_proxy = config.trust_proxy,
        persistent = config.db_path.is_some(),
        secret_rotation = config.previous_secret.is_some(),
        player = config.static_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "desativado".into()),
        "vellcast-relay iniciando"
    );
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("bind em {bind}"))?;
    serve_with_shutdown(Relay::from_config(config)?, listener, shutdown_signal()).await?;
    tracing::info!("vellcast-relay encerrado");
    Ok(())
}

fn init_logs() {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let json = std::env::var("VELLCAST_LOG_FORMAT").is_ok_and(|v| v == "json");
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}

/// Encerra com Ctrl+C ou SIGTERM (enviado por Docker/Dokploy).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = term => {}
    }
}
