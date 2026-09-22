//! The WebSocket+JSON control-plane server (yak canon-b46b).
//!
//! An [`axum`] app with one real route — `GET /ws` — that upgrades to a WebSocket and
//! runs [`handle_socket`]. Everything the server can do is a thin translation over the
//! core: client frames become [`canon_core::Command`]s or [`canon_core::ServiceSession`]
//! calls, and the authoritative [`canon_core::PlayerSnapshot`] stream is fanned out to
//! every connection. No queue, transport, or auth logic lives here or in a client — the
//! daemon's core is the single source of truth (the tideway lesson).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::{any, get};
use canon_core::{
    Command, ControlPlane, EntityId, Service, ServiceSession, SinkId, SourceRef, TrackMeta,
    TrackRef,
};

use crate::protocol::{ClientEnvelope, ClientMessage, PROTOCOL_VERSION, ReplyData, ServerMessage};

/// Shared server state: the control plane, plus a service session per music service.
pub struct AppState {
    control: Arc<dyn ControlPlane>,
    sessions: HashMap<Service, Arc<dyn ServiceSession>>,
}

impl AppState {
    /// Build state over a control plane (the bare player for a state-only mirror, or the
    /// daemon's playback controller for real audio), with no sessions yet.
    #[must_use]
    pub fn new(control: Arc<dyn ControlPlane>) -> Self {
        Self {
            control,
            sessions: HashMap::new(),
        }
    }

    /// Register a service session (keyed by [`ServiceSession::service`]). Builder-style
    /// so the daemon can wire Tidal today and more services later.
    #[must_use]
    pub fn with_session(mut self, session: Arc<dyn ServiceSession>) -> Self {
        self.sessions.insert(session.service(), session);
        self
    }

    fn session(&self, service: Service) -> Option<&Arc<dyn ServiceSession>> {
        self.sessions.get(&service)
    }
}

/// Build the axum router over shared state. Exposed for tests that bind their own
/// listener; the daemon uses [`serve`].
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/ws", any(ws_handler))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

/// Serve the control plane on `listener` until the process stops.
///
/// # Errors
/// Propagates the underlying `axum::serve` I/O error if the server exits abnormally.
pub async fn serve(state: Arc<AppState>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(state)).await
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// One connection: greet, push the current snapshot, then interleave snapshot pushes
/// with request handling until the socket closes.
async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut snapshots = state.control.subscribe();

    if send(
        &mut socket,
        &ServerMessage::Hello {
            protocol: PROTOCOL_VERSION,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    // Immediate full snapshot; `borrow_and_update` marks it seen so the change-watch
    // below only fires on the *next* transition (no duplicate first frame).
    let initial = {
        let snapshot = snapshots.borrow_and_update().clone();
        ServerMessage::Snapshot { snapshot }
    };
    if send(&mut socket, &initial).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            changed = snapshots.changed() => {
                if changed.is_err() {
                    break; // player actor gone
                }
                let message = {
                    let snapshot = snapshots.borrow_and_update().clone();
                    ServerMessage::Snapshot { snapshot }
                };
                if send(&mut socket, &message).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let reply = handle_text(text.as_str(), &state).await;
                        if send(&mut socket, &reply).await.is_err() {
                            break;
                        }
                    }
                    // axum auto-replies to pings; other frames are uninteresting here.
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
}

/// Parse one client frame and dispatch it, producing the reply to send back.
async fn handle_text(text: &str, state: &AppState) -> ServerMessage {
    match serde_json::from_str::<ClientEnvelope>(text) {
        Ok(envelope) => dispatch(envelope.message, envelope.id, state).await,
        Err(e) => ServerMessage::err(None, format!("bad request: {e}")),
    }
}

/// Translate one client operation into a core call and shape the reply.
async fn dispatch(message: ClientMessage, id: Option<u64>, state: &AppState) -> ServerMessage {
    match message {
        // --- transport: fire-and-forget into the player actor, ack immediately ---
        ClientMessage::Play => command(state, id, Command::Play).await,
        ClientMessage::Pause => command(state, id, Command::Pause).await,
        ClientMessage::Stop => command(state, id, Command::Stop).await,
        ClientMessage::Seek { position_ms } => {
            command(state, id, Command::Seek(Duration::from_millis(position_ms))).await
        }
        ClientMessage::SetVolume { volume } => command(state, id, Command::SetVolume(volume)).await,
        ClientMessage::SetMuted { muted } => command(state, id, Command::SetMuted(muted)).await,
        ClientMessage::SelectSink { sink } => {
            command(state, id, Command::SelectSink(SinkId(sink))).await
        }
        ClientMessage::Load { track } => command(state, id, Command::Load(*track)).await,
        ClientMessage::PlayTrack { service, track_id } => match track_ref(service, &track_id) {
            Some(track) => command(state, id, Command::Load(track)).await,
            None => ServerMessage::err(id, format!("cannot play a {service} source by id")),
        },
        ClientMessage::Enqueue { service, track_id } => match track_ref(service, &track_id) {
            Some(track) => command(state, id, Command::Enqueue(track)).await,
            None => ServerMessage::err(id, format!("cannot enqueue a {service} source by id")),
        },
        ClientMessage::Next => command(state, id, Command::Next).await,
        ClientMessage::Previous => command(state, id, Command::Previous).await,
        ClientMessage::Clear => command(state, id, Command::Clear).await,

        // --- service session / auth: request/response ---
        ClientMessage::LoginBegin { service } => {
            with_session(state, id, service, |session| async move {
                session.begin_login().await.map(ReplyData::DeviceCode)
            })
            .await
        }
        ClientMessage::LoginPoll { service } => {
            with_session(state, id, service, |session| async move {
                session
                    .poll_login()
                    .await
                    .map(|status| ReplyData::Login { status })
            })
            .await
        }
        ClientMessage::Account { service } => {
            with_session(state, id, service, |session| async move {
                session.account().await.map(ReplyData::Account)
            })
            .await
        }
    }
}

/// Build a minimal [`TrackRef`] from a service + track id (for play/enqueue). Only
/// id-based services are supported here (local files are addressed by path, not id).
fn track_ref(service: Service, id: &str) -> Option<TrackRef> {
    let source = match service {
        Service::Tidal => SourceRef::Tidal { id: id.to_string() },
        Service::Spotify => SourceRef::Spotify { id: id.to_string() },
        Service::Local => return None,
    };
    Some(TrackRef {
        id: EntityId::new(),
        meta: TrackMeta::default(),
        sources: vec![source],
    })
}

async fn command(state: &AppState, id: Option<u64>, cmd: Command) -> ServerMessage {
    match state.control.dispatch(cmd).await {
        Ok(()) => ServerMessage::ack(id),
        Err(e) => ServerMessage::err(id, e.to_string()),
    }
}

/// Resolve the target session (defaulting to Tidal) and run `f` against it, mapping the
/// result into a reply. Keeps every session verb's error handling in one place.
async fn with_session<F, Fut>(
    state: &AppState,
    id: Option<u64>,
    service: Option<Service>,
    f: F,
) -> ServerMessage
where
    F: FnOnce(Arc<dyn ServiceSession>) -> Fut,
    Fut: std::future::Future<Output = canon_core::Result<ReplyData>>,
{
    let service = service.unwrap_or(Service::Tidal);
    let Some(session) = state.session(service) else {
        return ServerMessage::err(id, format!("no session registered for {service}"));
    };
    match f(Arc::clone(session)).await {
        Ok(data) => ServerMessage::ok(id, data),
        Err(e) => ServerMessage::err(id, e.to_string()),
    }
}

/// Encode and send one server frame. Server messages are plain serde types, so
/// serialization is infallible; a send error means the socket is gone.
async fn send(socket: &mut WebSocket, message: &ServerMessage) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).expect("server messages serialize");
    socket.send(Message::Text(text.into())).await
}
