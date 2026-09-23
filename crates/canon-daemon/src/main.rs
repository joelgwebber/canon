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

mod control;
mod controller;

use canon_api::{AppState, serve};
use canon_core::{ControlPlane, LoginStatus, PlayerHandle, Quality, Service, ServiceSession};
use canon_tidal::{TidalSession, TokenStore, WreqHttp};
use clap::{Parser, Subcommand};

use controller::PlaybackController;

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
        /// Use the PKCE (browser-redirect) flow instead of device-code. Required for
        /// playback: Tidal no longer lets device-code tokens stream.
        #[arg(long)]
        pkce: bool,
        /// Step 2 of the PKCE flow: the URL you were redirected to after logging in
        /// (contains `?code=`). Omit to run step 1, which prints the login URL.
        #[arg(long, requires = "pkce")]
        redirect: Option<String>,
    },
    /// Resolve a Tidal track and play it on the default output device (needs a PKCE
    /// login). The end-to-end path: Source::resolve -> decode -> ring -> cpal.
    Play {
        /// Tidal track id.
        track_id: String,
        #[arg(long, value_enum, default_value_t = QualityArg::Lossless)]
        quality: QualityArg,
    },
    /// Interactive keypress client over a running `canon serve` — enqueue tracks and
    /// drive transport/queue/seek to stress-test.
    Control {
        /// Track ids to enqueue on connect (also the 1-9 keypad palette).
        track_ids: Vec<String>,
        /// Address of the running daemon's control plane.
        #[arg(long, default_value = "127.0.0.1:7345")]
        connect: String,
    },
    /// Resolve a Tidal track's stream (diagnostic): print manifest type, codec, and the
    /// resolved segment URLs.
    Resolve {
        /// Tidal track id.
        track_id: String,
        /// Requested quality (clamped to what the account/client can serve).
        #[arg(long, value_enum, default_value_t = QualityArg::Lossless)]
        quality: QualityArg,
    },
    /// Decode a local audio file and play it on the default output device. Proves the
    /// decode -> ring -> cpal path end to end (canon-8629/canon-940d).
    PlayFile {
        /// Path to a FLAC/AAC/MP4 file.
        path: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum QualityArg {
    Low,
    High,
    Lossless,
    Hires,
}

impl From<QualityArg> for Quality {
    fn from(q: QualityArg) -> Self {
        match q {
            QualityArg::Low => Quality::Low,
            QualityArg::High => Quality::High,
            QualityArg::Lossless => Quality::Lossless,
            QualityArg::Hires => Quality::HiRes,
        }
    }
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
            pkce,
            redirect,
        } => {
            if pkce {
                run_login_pkce(&state_dir, redirect).await
            } else {
                run_login(&state_dir).await
            }
        }
        Cmd::Play { track_id, quality } => run_play(&state_dir, &track_id, quality.into()).await,
        Cmd::Control { track_ids, connect } => control::run(&connect, track_ids).await,
        Cmd::Resolve { track_id, quality } => {
            run_resolve(&state_dir, &track_id, quality.into()).await
        }
        Cmd::PlayFile { path } => run_play_file(path).await,
    }
}

/// Decode a local file and play it on the default output device — proves the
/// decode -> ring -> cpal path (canon-8629/canon-940d) independently of any source.
async fn run_play_file(path: PathBuf) -> Result<(), BoxError> {
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    let file = std::fs::File::open(&path)?;
    let input: Box<dyn canon_core::MediaInput> = Box::new(file);
    let clock = std::sync::Arc::new(canon_core::FrameClock::new());

    println!("playing {} …", path.display());
    // play_blocking parks on the ring and sleeps; keep it off the async runtime.
    let stats = tokio::task::spawn_blocking(move || {
        canon_audio::play_blocking(input, extension.as_deref(), clock)
    })
    .await??;
    println!(
        "done: {} frames at {} Hz, {} ch",
        stats.frames_played, stats.sample_rate, stats.channels
    );
    Ok(())
}

/// Resolve a track's stream and print what came back — a diagnostic for the Tidal
/// stream-resolution path (playbackinfo -> manifest -> segment URLs).
async fn run_resolve(
    state_dir: &std::path::Path,
    track_id: &str,
    quality: Quality,
) -> Result<(), BoxError> {
    let session = build_tidal_session(state_dir).await?;
    if !session.is_authenticated() {
        return Err("not logged in — run `canon login tidal` first".into());
    }
    let resolved = session.resolve_stream(track_id, quality).await?;
    println!("resolved track {track_id}:");
    println!("  codec:          {:?}", resolved.codec);
    println!("  served quality: {:?}", resolved.served_quality);
    println!(
        "  sample rate:    {} Hz, bit depth: {:?}",
        resolved.info.sample_rate, resolved.info.bit_depth
    );
    println!("  extension hint: {}", resolved.extension_hint);
    println!(
        "  segments:       {} media (+{} init)",
        resolved.media_urls.len(),
        resolved.init_url.is_some() as u8
    );
    let total: f64 = resolved.segment_secs.iter().sum();
    if total > 0.0 {
        println!("  timeline:       {total:.1}s across the segment timeline");
    }
    if let Some(first) = resolved.init_url.as_ref().or(resolved.media_urls.first()) {
        let host = first.split('/').nth(2).unwrap_or("?");
        println!("  first segment:  {host}");
    }
    Ok(())
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

    // The controller turns commands into real audio (resolve -> engine -> player).
    let controller = PlaybackController::new(player.clone(), session.clone(), Quality::Lossless);
    let control: Arc<dyn ControlPlane> = controller;
    let state = Arc::new(AppState::new(control).with_session(session));

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

/// Drive the PKCE (browser-redirect) login: print the login URL, read the pasted
/// redirect URL, exchange it, and verify with an authenticated call. This is the
/// streaming-capable path.
async fn run_login_pkce(
    state_dir: &std::path::Path,
    redirect: Option<String>,
) -> Result<(), BoxError> {
    let session = build_tidal_session(state_dir).await?;

    let Some(redirect) = redirect else {
        // Step 1: print the login URL and park the challenge for step 2.
        let url = session.pkce_login_url().await?;
        println!(
            "\nTo authorize canon with Tidal (streaming):\n\
             \n  1. Open this URL in a browser and log in:\n\n     {url}\n\
             \n  2. You'll land on a blank or \"Oops\" page. Copy that page's URL from the\n\
             \x20    address bar (it contains ...?code=...) and run:\n\
             \n     canon login tidal --pkce --redirect '<that URL>'\n"
        );
        return Ok(());
    };

    // Step 2: complete the exchange and verify with an authenticated call.
    session.complete_pkce_login(&redirect).await?;
    let account = session.account().await?;
    let country = account
        .attributes
        .get("country_code")
        .map(String::as_str)
        .unwrap_or("?");
    println!(
        "\nLogged in to Tidal (PKCE) as user {} (country {}). Streaming tokens saved.",
        account.user_id, country
    );
    Ok(())
}

/// Resolve a Tidal track and play it on the default output device — the whole path.
async fn run_play(
    state_dir: &std::path::Path,
    track_id: &str,
    quality: Quality,
) -> Result<(), BoxError> {
    use canon_core::{Codec, EngineEvent, FrameClock};

    let session = build_tidal_session(state_dir).await?;
    if !session.is_authenticated() {
        return Err("not logged in — run `canon login tidal --pkce` first".into());
    }

    println!("resolving track {track_id} …");
    let resolved = session.open_stream(track_id, quality).await?;
    let codec = resolved.info.codec;
    let hint = match codec {
        Codec::Flac => Some("flac".to_owned()),
        Codec::Aac | Codec::Alac => Some("m4a".to_owned()),
        _ => None,
    };
    println!("streaming (codec {codec:?}) …");

    // Drive the streaming engine directly and wait for it to finish.
    let clock = std::sync::Arc::new(FrameClock::new());
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<EngineEvent>();
    let _audio = canon_audio::AudioPlayer::start(
        resolved.input,
        hint,
        clock,
        events_tx,
        0,
        canon_audio::Output::Local,
    );
    while let Some(event) = events_rx.recv().await {
        match event {
            EngineEvent::Loaded { sample_rate, .. } => println!("playing at {sample_rate} Hz …"),
            EngineEvent::Ended => {
                println!("done.");
                break;
            }
            EngineEvent::Failed(message) => return Err(message.into()),
            _ => {}
        }
    }
    Ok(())
}

/// Build a Tidal session over the browser-impersonation HTTP client, restoring any
/// persisted tokens from `<state_dir>/tidal.json`. Returns the concrete type so callers
/// can use both its [`ServiceSession`] and [`canon_core::Source`] faces; it coerces to
/// `Arc<dyn ServiceSession>` where the API wants that.
async fn build_tidal_session(state_dir: &std::path::Path) -> Result<Arc<TidalSession>, BoxError> {
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
    if let Some(dirs) = directories::ProjectDirs::from("", "", "canon") {
        return dirs.data_dir().to_path_buf();
    }
    PathBuf::from(".canon")
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // Quiet Symphonia's benign per-open chatter (the "skipped 4 bytes of junk"
                // probe note on every fMP4 open, and the isomp4 demuxer INFO lines).
                tracing_subscriber::EnvFilter::new(
                    "info,symphonia_core::formats::probe=error,symphonia_format_isomp4=warn",
                )
            }),
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
