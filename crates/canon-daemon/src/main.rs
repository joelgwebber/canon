//! canon-daemon — the headless binary (`canon`).
//!
//! Hosts the long-lived task set and supervises its lifecycle. Today that is the
//! player state core plus a snapshot logger; the Tidal source, audio pipeline, sinks
//! and discovery, the library, and the ws+json control API attach here as they land.
//! The daemon owns startup, graceful shutdown on SIGINT/SIGTERM, and teardown order.

use canon_core::PlayerHandle;

#[tokio::main]
async fn main() {
    init_tracing();
    tracing::info!("canon daemon starting");

    // The single source of truth for playback. Everything else drives it via commands
    // (API/MCP) or engine events (audio/sink).
    let player = PlayerHandle::spawn();

    // A supervised task: mirror the authoritative snapshot stream into the log. This
    // is the template every future subsystem task follows — subscribe to the core,
    // react, and exit cleanly when the actor goes away.
    let snapshot_logger = tokio::spawn(log_snapshots(player.subscribe()));

    tracing::info!(snapshot = ?player.snapshot(), "player state core online");

    shutdown_signal().await;
    tracing::info!("shutdown signal received; stopping");

    // Dropping the last handle stops the actor; stop our watcher too.
    snapshot_logger.abort();
    drop(player);
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
}

/// Log the initial snapshot and every subsequent change until the actor stops.
async fn log_snapshots(mut snapshots: tokio::sync::watch::Receiver<canon_core::PlayerSnapshot>) {
    loop {
        {
            let snapshot = snapshots.borrow_and_update();
            tracing::info!(
                seq = snapshot.seq,
                state = ?snapshot.state,
                position_ms = snapshot.position_ms,
                "player state"
            );
        }
        if snapshots.changed().await.is_err() {
            break; // actor gone
        }
    }
}

/// Resolve when the process is asked to stop (Ctrl-C, or SIGTERM on unix).
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
