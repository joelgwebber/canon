//! canon-daemon — the headless binary (`canon`).
//!
//! Two entry points over one wiring:
//! * `canon` / `canon serve` — run the daemon: spawn the player state core, restore the
//!   Tidal session from disk, and serve the WebSocket+JSON control plane so UIs and
//!   agents can drive playback and log sources in.
//! * `canon login <service>` — run the OAuth device-code flow to completion at the
//!   terminal (print the code + URL, poll, persist tokens) and make the first
//!   authenticated call. This is the human-in-the-loop half the ws API can't automate.
//!
//! Both share [`build_tidal_session`], so a token minted by `login` is exactly what
//! `serve` picks up — the end-to-end path from auth to an authenticated Tidal call.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use canon_api::{AppState, serve};
use canon_core::{LoginStatus, PlayerHandle, Service, ServiceSession};
use canon_tidal::{TidalSession, TokenStore, WreqHttp};
use clap::{Parser, Subcommand};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// canon — a headless music daemon.
#[derive(Debug, Parser)]
#[command(name = "canon", version, about)]
struct Cli {
    /// Directory for tokens and other daemon state. Defaults to the platform data dir
    /// (override also via `CANON_STATE_DIR`).
    #[arg(long, global = true, env = "CANON_STATE_DIR")]
    state_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// Run the daemon and serve the control plane (the default).
    Serve {
        /// Address for the WebSocket+JSON control plane.
        #[arg(long, default_value = "127.0.0.1:7345")]
        bind: String,
    },
    /// Log a music service in via the OAuth device-code flow, then verify with an
    /// authenticated call.
    Login {
        /// Which service to log in (currently only `tidal`).
        #[arg(value_enum, default_value_t = LoginService::Tidal)]
        service: LoginService,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum LoginService {
    Tidal,
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let cli = Cli::parse();
    init_tracing();
    let state_dir = resolve_state_dir(cli.state_dir);

    match cli.command.unwrap_or(Cmd::Serve {
        bind: "127.0.0.1:7345".to_string(),
    }) {
        Cmd::Serve { bind } => run_serve(&state_dir, &bind).await,
        Cmd::Login {
            service: LoginService::Tidal,
        } => run_login(&state_dir).await,
    }
}

/// Run the player + control plane until a shutdown signal arrives.
async fn run_serve(state_dir: &std::path::Path, bind: &str) -> Result<(), BoxError> {
    tracing::info!("canon daemon starting");

    // The single source of truth for playback; everything drives it via the API.
    let player = PlayerHandle::spawn();

    // Restore the Tidal session (unauthenticated until `canon login tidal` has run).
    let session = build_tidal_session(state_dir).await?;
    if session.is_authenticated() {
        tracing::info!("tidal session restored from disk");
    } else {
        tracing::info!("no tidal session yet — run `canon login tidal`");
    }

    let state = Arc::new(AppState::new(player.clone()).with_session(session));

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(addr = %listener.local_addr()?, "control plane listening (ws /ws)");

    // Mirror the authoritative snapshot stream into the log — the template every future
    // subsystem task follows: subscribe to the core, react, exit when the actor stops.
    let snapshot_logger = tokio::spawn(log_snapshots(player.subscribe()));

    tokio::select! {
        result = serve(state, listener) => {
            result?;
        }
        () = shutdown_signal() => {
            tracing::info!("shutdown signal received; stopping");
        }
    }

    snapshot_logger.abort();
    Ok(())
}

/// Drive the device-code login to completion at the terminal, then prove it with an
/// authenticated call.
async fn run_login(state_dir: &std::path::Path) -> Result<(), BoxError> {
    let session = build_tidal_session(state_dir).await?;

    let code = session.begin_login().await?;
    println!("\nTo authorize canon with Tidal:");
    if let Some(complete) = &code.verification_uri_complete {
        println!("  open {complete}");
    }
    println!(
        "  or visit https://{} and enter code: {}\n",
        code.verification_uri.trim_start_matches("https://"),
        code.user_code
    );
    println!(
        "Waiting for approval (this code expires in {}s)…",
        code.expires_in
    );

    let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
    let mut interval = code.interval.max(1);
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("device code expired before approval".into());
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;
        match session.poll_login().await? {
            LoginStatus::Pending => {}
            LoginStatus::SlowDown => interval += 2,
            LoginStatus::Authorized => break,
        }
    }

    let account = session.account().await?;
    let country = account
        .attributes
        .get("country_code")
        .map(String::as_str)
        .unwrap_or("?");
    println!(
        "\nLogged in to Tidal as user {} (country {}). Tokens saved.",
        account.user_id, country
    );
    Ok(())
}

/// Build a Tidal session over the browser-impersonation HTTP client, restoring any
/// persisted tokens from `<state_dir>/tidal.json`.
async fn build_tidal_session(
    state_dir: &std::path::Path,
) -> Result<Arc<dyn ServiceSession>, BoxError> {
    let http = Arc::new(WreqHttp::chrome_android()?);
    let store = TokenStore::new(state_dir.join(token_file(Service::Tidal)));
    let session = TidalSession::restore(http, store).await?;
    Ok(Arc::new(session))
}

fn token_file(service: Service) -> String {
    format!("{service}.json")
}

/// Resolve the state directory: an explicit flag/env wins, else the platform data dir,
/// else a `.canon` fallback in the current directory.
fn resolve_state_dir(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = explicit {
        return dir;
    }
    if let Some(dirs) = directories::ProjectDirs::from("", "canon", "canon") {
        return dirs.data_dir().to_path_buf();
    }
    PathBuf::from(".canon")
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
