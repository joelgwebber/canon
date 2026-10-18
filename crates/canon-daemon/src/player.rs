//! `canon player` (canon-1ad8, canon-7196): a player that carries out its own playback against a
//! remote server.
//!
//! The server keeps the player's queue and decides what plays; this process produces the sound,
//! on this machine's output or the speakers on its own LAN. It is one connection with two roles:
//! its player's executor (orders down, reports up) and, unless `--headless`, the TUI controlling
//! the same player. The executor is the same controller `canon serve` runs for its own players,
//! fetching each track from the server instead of from a music service, so a player needs no
//! service logins: the server holds those.
//!
//! Discovery, the stream server for speakers, and the firewall rules they need belong to this
//! machine (docs/firewall.md); the server only needs to be reachable.

use std::sync::Arc;
use std::time::Duration;

use canon_core::ExecutorReport;
use canon_protocol::{ClientEnvelope, ClientMessage, ServerMessage};
use canon_tui::{Endpoint, Incoming, Link};
use tokio::sync::mpsc;

use crate::BoxError;
use crate::controller::{PlaybackController, StreamHost, Tracks};
use crate::remote::RemoteAudio;

/// How long to wait before reconnecting to the server, at first and at most.
const RECONNECT_FIRST: Duration = Duration::from_secs(1);
const RECONNECT_MAX: Duration = Duration::from_secs(30);

/// Run a player against the server at `endpoint`, until the user quits the TUI or, headless,
/// until the process is stopped.
pub async fn run(
    endpoint: Endpoint,
    headless: bool,
    script: Option<toque::DriverOpts>,
    lan_port: u16,
) -> Result<(), BoxError> {
    let discovery = match canon_sink::DiscoveryService::spawn(lan_port) {
        Ok(discovery) => Some(Arc::new(discovery)),
        Err(e) => {
            tracing::warn!("renderer discovery unavailable: {e}; playing locally only");
            None
        }
    };
    let streams = Arc::new(StreamHost::new(lan_port));
    let settings: Arc<dyn canon_core::SettingsStore> = Arc::new(Defaults);
    let server = endpoint.addr.clone();
    let make_executor = move |orders, reports| {
        PlaybackController::new(
            orders,
            reports,
            Tracks::Remote(RemoteAudio::new(server.clone())),
            Arc::clone(&settings),
            discovery.clone(),
            Arc::clone(&streams),
        )
    };

    if headless {
        let mut wait = RECONNECT_FIRST;
        loop {
            match serve_once(&endpoint, &make_executor).await {
                Ok(()) => wait = RECONNECT_FIRST,
                Err(e) => tracing::warn!("{e}"),
            }
            tracing::info!("reconnecting to the server in {}s", wait.as_secs());
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(RECONNECT_MAX);
        }
    }

    // With a UI: one connection, the TUI on it, the executor beside it. Quitting the TUI ends
    // the connection, and with it this player's playback.
    let (ui, link) = connect(&endpoint, &make_executor).await?;
    let result = match script {
        // The same TUI driven from a script (toque's protocol on stdin), for checking a player
        // with a UI end to end, the executor sharing its connection.
        Some(opts) => {
            let runtime = tokio::runtime::Handle::current();
            tokio::task::spawn_blocking(move || {
                let app = canon_tui::App::new(std::time::Instant::now());
                toque::run(canon_tui::Headless::new(app, link, runtime), opts)
                    .map_err(|e| e.to_string())
            })
            .await?
        }
        None => canon_tui::run_on(link).await,
    };
    drop(ui);
    result.map_err(Into::into)
}

/// A player's settings: the defaults (flow mode off on every output). Per-output preferences live
/// in the server's settings, which a player doesn't read yet.
struct Defaults;

#[async_trait::async_trait]
impl canon_core::SettingsStore for Defaults {
    fn get(&self) -> canon_core::Settings {
        canon_core::Settings::default()
    }
    async fn set(&self, _settings: canon_core::Settings) -> canon_core::Result<()> {
        Err(canon_core::Error::Unsupported(
            "a player's settings are the server's".into(),
        ))
    }
}

type MakeExecutor = dyn Fn(
    mpsc::UnboundedReceiver<canon_core::ExecutorOrder>,
    mpsc::UnboundedSender<ExecutorReport>,
) -> Arc<PlaybackController>;

/// Connect, become the player's executor, and start one: the executor's half (dropping it ends
/// the executor), and the link for a UI.
async fn connect(
    endpoint: &Endpoint,
    make_executor: &MakeExecutor,
) -> Result<(Executing, Link), BoxError> {
    let (orders_tx, orders) = mpsc::unbounded_channel();
    let link = canon_tui::connect_with(endpoint, Some(orders_tx)).await?;
    // The first frames are the greeting and the snapshot; the app reads them too, so they are
    // left for it. Becoming the executor is a request like any other.
    link.outgoing
        .send(ClientEnvelope {
            id: None,
            message: ClientMessage::BecomeExecutor,
        })
        .map_err(|_| "the connection closed at once")?;
    let (reports_tx, mut reports) = mpsc::unbounded_channel();
    let controller = make_executor(orders, reports_tx);
    let outgoing = link.outgoing.clone();
    let forward = tokio::spawn(async move {
        while let Some(report) = reports.recv().await {
            let envelope = ClientEnvelope {
                id: None,
                message: ClientMessage::Executor { report },
            };
            if outgoing.send(envelope).is_err() {
                break;
            }
        }
    });
    tracing::info!("carrying out this player's playback");
    Ok((
        Executing {
            _controller: controller,
            forward,
        },
        link,
    ))
}

/// A running executor: its controller, and the task passing its reports to the server.
struct Executing {
    _controller: Arc<PlaybackController>,
    forward: tokio::task::JoinHandle<()>,
}

impl Drop for Executing {
    fn drop(&mut self) {
        self.forward.abort();
    }
}

/// Headless: connect, carry out playback until the connection goes, and return why.
async fn serve_once(endpoint: &Endpoint, make_executor: &MakeExecutor) -> Result<(), BoxError> {
    let (executing, mut link) = connect(endpoint, make_executor).await?;
    let why = loop {
        match link.incoming.recv().await {
            Some(Incoming::Message(message)) => {
                if let ServerMessage::Reply {
                    ok: false, error, ..
                } = *message
                {
                    tracing::warn!("the server refused: {}", error.unwrap_or_default());
                }
            }
            Some(Incoming::Unreadable(what)) => {
                tracing::warn!("unreadable from the server: {what}")
            }
            Some(Incoming::Closed(why)) => break why,
            None => break "the connection task ended".into(),
        }
    };
    drop(executing);
    Err(format!("lost the server: {why}").into())
}
