//! canon-daemon — the headless binary (`canon`).
//!
//! Two entry points over one wiring:
//! * `canon` / `canon serve` — run the daemon: spawn the player state core, restore the
//!   service connections from disk, and serve the WebSocket+JSON control plane so UIs and
//!   agents can drive playback and sign services in.
//! * `canon login <service>` — sign in at the terminal and make the first authenticated call.
//!
//! Both go through [`tidal_connector`], so credentials minted by `login` are exactly what
//! `serve` picks up.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod autoplay;
mod control;
mod controller;
mod settings;

use canon_api::{AppState, serve};
use canon_core::{
    Capability, Connector, ControlPlane, LoginStatus, PlayerHandle, Quality, Sources,
};
use canon_tidal::{TidalConnector, WreqHttp};
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
    /// Line-oriented client over a running `canon serve`: one command per line on stdin,
    /// so it drives equally well from a terminal or a pipe.
    ///
    /// `printf 'sink Tunes\nenqueue 520285418\nsleep 40\n' | canon control`
    Control {
        /// Track ids to enqueue on connect.
        track_ids: Vec<String>,
        /// Address of the running daemon's control plane.
        #[arg(long, default_value = "127.0.0.1:7345")]
        connect: String,
        /// Print every server frame as one line of JSON instead of a readable summary.
        #[arg(long)]
        json: bool,
        /// Print state transitions only, suppressing the once-a-second position echo.
        #[arg(long)]
        quiet: bool,
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
    /// An authenticated GET of any Tidal API path, printed as JSON (diagnostic): for reading an
    /// endpoint's real shape before modelling it. The country code is added for you.
    TidalGet {
        /// The API path, e.g. `/v1/tracks/33348478`.
        path: String,
        /// Extra query parameters as `key=value`.
        query: Vec<String>,
    },
    /// Decode a local audio file and play it on the default output device. Proves the
    /// decode -> ring -> cpal path end to end (canon-8629/canon-940d).
    PlayFile {
        /// Path to a FLAC/AAC/MP4 file.
        path: PathBuf,
    },
    /// Browse the LAN for network renderers (Chromecast now, DLNA later) and print what the
    /// discovery supervisor finds. The first on-metal check that a real device is seen.
    Devices {
        /// How long to browse before printing, in seconds.
        #[arg(long, default_value_t = 4)]
        secs: u64,
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
        Cmd::Control {
            track_ids,
            connect,
            json,
            quiet,
        } => control::run(&connect, track_ids, json, quiet).await,
        Cmd::Resolve { track_id, quality } => {
            run_resolve(&state_dir, &track_id, quality.into()).await
        }
        Cmd::TidalGet { path, query } => run_tidal_get(&state_dir, &path, &query).await,
        Cmd::PlayFile { path } => run_play_file(path).await,
        Cmd::Devices { secs } => run_devices(secs).await,
    }
}

/// Browse the LAN for renderers and print the discovery supervisor's snapshot. This is the
/// diagnostic that answers "does canon see my speaker?" before any casting is attempted.
async fn run_devices(secs: u64) -> Result<(), BoxError> {
    let discovery =
        canon_sink::DiscoveryService::spawn().map_err(|e| format!("start discovery: {e}"))?;
    println!("browsing for renderers for {secs}s …");
    tokio::time::sleep(std::time::Duration::from_secs(secs)).await;

    let devices = discovery.devices().borrow().clone();
    if devices.is_empty() {
        println!("no renderers found. check that the device is on and on this LAN subnet.");
    } else {
        println!("found {} renderer(s):", devices.len());
        for device in &devices {
            println!(
                "  {:<11} {:<24} {}  [{}]",
                format!("{:?}", device.kind),
                device.name,
                device.addr,
                device.id.0
            );
            if let Some(location) = &device.location {
                println!("  {:<11} described at {location}", "");
            }
        }
    }
    Ok(())
}

/// Decode a local file and play it on the default output device — proves the
/// decode -> ring -> cpal path (canon-8629/canon-940d) independently of any source.
async fn run_play_file(path: PathBuf) -> Result<(), BoxError> {
    use canon_core::EngineEvent;

    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_owned);
    let input: Box<dyn canon_core::MediaInput> = Box::new(std::fs::File::open(&path)?);
    let clock = Arc::new(canon_core::FrameClock::new());
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();

    println!("playing {} …", path.display());
    // The same engine the daemon plays through, on the local output, with nothing but this loop
    // listening to it.
    let _audio = canon_audio::AudioPlayer::start(
        input,
        extension,
        clock,
        events_tx,
        0,
        canon_audio::Output::Local,
    );
    while let Some(event) = events.recv().await {
        match event {
            EngineEvent::Loaded { sample_rate, .. } => println!("output at {sample_rate} Hz"),
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

/// Resolve a track's stream and print what came back — a diagnostic for the Tidal
/// stream-resolution path (playbackinfo -> manifest -> segment URLs).
async fn run_tidal_get(
    state_dir: &std::path::Path,
    path: &str,
    query: &[String],
) -> Result<(), BoxError> {
    let connector = tidal_connector(state_dir).await?;
    let session = signed_in(&connector, Capability::Catalog)?;
    let query: Vec<(&str, &str)> = query
        .iter()
        .map(|pair| pair.split_once('=').ok_or("query parameters are key=value"))
        .collect::<Result<_, _>>()?;
    let reply = session.get_json(path, &query).await?;
    println!("{}", serde_json::to_string_pretty(&reply)?);
    Ok(())
}

async fn run_resolve(
    state_dir: &std::path::Path,
    track_id: &str,
    quality: Quality,
) -> Result<(), BoxError> {
    let connector = tidal_connector(state_dir).await?;
    let session = signed_in(&connector, Capability::Stream)?;
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

    // The single source of truth for playback; everything drives it via the API. Its decisions
    // come out as effects, which the controller below carries out.
    let (player, effects) = PlayerHandle::spawn_with_effects();

    // Restore the service connections. Each login method keeps its own credentials, and what a
    // connection grants decides what it is used for.
    let tidal = tidal_connector(state_dir).await?;
    for capability in [Capability::Stream, Capability::Catalog] {
        if tidal.grants(capability) {
            tracing::info!("tidal can {capability}");
        } else {
            tracing::info!("tidal can't {capability}: {}", tidal.hint(capability));
        }
    }

    // Confirm with Tidal that the streaming login really streams, off the startup path: a login
    // Tidal has stopped letting play is then routed around, and says so, before anyone presses
    // play.
    tokio::spawn({
        let tidal = Arc::clone(&tidal);
        async move { tidal.probe().await }
    });

    // Where tracks come from, shared by playback and by the library, which describes new ones.
    let sources = Sources::new().with_connector(tidal);

    // The library: every track a client names becomes (or already is) one of its entities.
    let library_path = state_dir.join("library.sqlite");
    let library = canon_library::Library::open(&library_path)
        .await
        .map_err(|e| format!("open the library at {}: {e}", library_path.display()))?;

    // The user's settings. A file that doesn't parse stops the daemon here, saying where it is.
    let settings = Arc::new(settings::FileSettings::load(&state_dir.join("settings.json")).await?);
    let settings_store: Arc<dyn canon_core::SettingsStore> = settings.clone();

    // LAN renderer discovery, so clients can list and select network sinks. A discovery failure
    // is not fatal: local playback must still work (and on macOS discovery needs a permission
    // grant the daemon can't obtain for itself).
    let discovery = match canon_sink::DiscoveryService::spawn() {
        Ok(discovery) => {
            tracing::info!("renderer discovery started");
            Some(Arc::new(discovery))
        }
        Err(e) => {
            tracing::warn!("renderer discovery unavailable: {e}");
            None
        }
    };

    // The controller turns commands into real audio (resolve -> engine -> player).
    let controller = PlaybackController::new(
        player.clone(),
        effects,
        sources.clone(),
        Quality::Lossless,
        settings.clone(),
        discovery,
    );
    let control: Arc<dyn ControlPlane> = controller;
    let state = Arc::new(
        AppState::new(control)
            .with_settings(settings)
            .with_library(library.clone(), sources.clone()),
    );

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(addr = %listener.local_addr()?, "control plane listening (ws /ws)");

    // Mirror the authoritative snapshot stream into the log — the template every future
    // subsystem task follows: subscribe to the core, react, exit when the actor stops.
    let snapshot_logger = tokio::spawn(log_snapshots(player.subscribe()));
    let autoplay = autoplay::spawn(player.clone(), library, sources, settings_store);

    tokio::select! {
        result = serve(state, listener) => {
            result?;
        }
        () = shutdown_signal() => {
            tracing::info!("shutdown signal received; stopping");
        }
    }

    snapshot_logger.abort();
    autoplay.abort();
    Ok(())
}

/// Drive the device-code login to completion at the terminal, then prove it with an
/// authenticated call.
async fn run_login(state_dir: &std::path::Path) -> Result<(), BoxError> {
    let connector = tidal_connector(state_dir).await?;
    let session = connector.session(canon_tidal::connector::DEVICE)?;

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
    let connector = tidal_connector(state_dir).await?;
    let session = connector.session(canon_tidal::connector::PKCE)?;

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

/// Tidal's connector over the browser-impersonation HTTP client, restoring each login method's
/// credentials from the state directory.
async fn tidal_connector(state_dir: &std::path::Path) -> Result<Arc<TidalConnector>, BoxError> {
    let http = Arc::new(WreqHttp::chrome_android()?);
    Ok(Arc::new(TidalConnector::restore(http, state_dir).await?))
}

/// The signed-in Tidal session for `need`, or what to do about there being none.
fn signed_in(
    connector: &TidalConnector,
    need: Capability,
) -> Result<Arc<canon_tidal::TidalSession>, BoxError> {
    connector
        .session_for(need)
        .cloned()
        .ok_or_else(|| format!("Tidal can't {need}: {}", connector.hint(need)).into())
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
