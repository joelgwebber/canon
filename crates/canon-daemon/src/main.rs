//! canon-daemon — the headless binary (`canon`).
//!
//! Eventually this wires together the supervised task set: sources, the player state
//! core, the audio pipeline, sinks + discovery, the library, and the ws+json control
//! API (yaks canon-16b7 for lifecycle, canon-e284 for the state core). For now it only
//! starts the runtime and exercises the core types, proving the workspace seams
//! compile end to end.

use canon_core::PlayerSnapshot;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("canon daemon starting");

    // Placeholder: construct the initial authoritative snapshot to exercise the core
    // types. The real supervised task set replaces this.
    let snapshot = PlayerSnapshot::idle();
    tracing::info!(?snapshot, "initial player state");

    // TODO(canon-4a94): start + supervise the long-lived task set, await shutdown.
}
