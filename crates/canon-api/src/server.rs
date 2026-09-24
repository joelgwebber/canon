//! The WebSocket+JSON control-plane server (yak canon-b46b).
//!
//! An [`axum`] app with one real route — `GET /ws` — that upgrades to a WebSocket and
//! runs [`handle_socket`]. Everything the server can do is a thin translation over the
//! core: client frames become [`canon_core::Command`]s, library calls or
//! [`canon_core::Connector`] calls, and the authoritative [`canon_core::PlayerSnapshot`] stream is fanned out to
//! every connection. No queue, transport, or auth logic lives here or in a client — the
//! daemon's core is the single source of truth (the tideway lesson).

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::routing::{any, get};
use canon_core::{
    Command, Connector, ControlPlane, Service, SettingsStore, SinkId, SourceRef, Sources,
};
use canon_library::Library;

use crate::protocol::{
    ClientEnvelope, ClientMessage, PROTOCOL_VERSION, QueueAt, ReplyData, ServerMessage, ServiceView,
};

/// Results per kind a search returns when the client doesn't say.
const SEARCH_LIMIT: usize = 10;
/// Entries per page of the saved library when the client doesn't say.
const LIBRARY_PAGE: usize = 50;

/// Shared server state: the control plane, plus a service session per music service.
pub struct AppState {
    control: Arc<dyn ControlPlane>,
    settings: Option<Arc<dyn SettingsStore>>,
    /// Where track ids from clients become canon entities, and the sources that describe them.
    library: Option<(Library, Sources)>,
}

impl AppState {
    /// Build state over a control plane (the bare player for a state-only mirror, or the
    /// daemon's playback controller for real audio), with no sessions yet.
    #[must_use]
    pub fn new(control: Arc<dyn ControlPlane>) -> Self {
        Self {
            control,
            settings: None,
            library: None,
        }
    }

    /// Resolve the tracks clients name through `library`, describing new ones with `sources`.
    #[must_use]
    pub fn with_library(mut self, library: Library, sources: Sources) -> Self {
        self.library = Some((library, sources));
        self
    }

    /// Serve the user's settings from `store`.
    #[must_use]
    pub fn with_settings(mut self, store: Arc<dyn SettingsStore>) -> Self {
        self.settings = Some(store);
        self
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
        ClientMessage::ListSinks => ServerMessage::ok(
            id,
            ReplyData::Sinks {
                sinks: state.control.sinks(),
            },
        ),
        ClientMessage::Queue => {
            let queue = state.control.queue();
            ServerMessage::ok(
                id,
                ReplyData::Queue {
                    revision: queue.revision,
                    index: queue.index,
                    tracks: queue.tracks.as_ref().clone(),
                },
            )
        }
        ClientMessage::Settings => match &state.settings {
            Some(store) => ServerMessage::ok(
                id,
                ReplyData::Settings {
                    settings: store.get(),
                },
            ),
            None => ServerMessage::err(id, "settings are unavailable"),
        },
        ClientMessage::SetSettings { settings } => match &state.settings {
            Some(store) => match store.set(settings).await {
                Ok(()) => ServerMessage::ack(id),
                Err(e) => ServerMessage::err(id, e.to_string()),
            },
            None => ServerMessage::err(id, "settings are unavailable"),
        },
        ClientMessage::PlayTrack { service, track_id } => {
            match track(state, service, &track_id).await {
                Ok(track) => command(state, id, Command::Load(track)).await,
                Err(e) => ServerMessage::err(id, e),
            }
        }
        ClientMessage::Enqueue { service, track_id } => {
            match track(state, service, &track_id).await {
                Ok(track) => command(state, id, Command::Enqueue(track)).await,
                Err(e) => ServerMessage::err(id, e),
            }
        }
        ClientMessage::QueueAdd { items, at, start } => {
            let Some((library, sources)) = &state.library else {
                return ServerMessage::err(id, "the library is unavailable");
            };
            let tracks = match library.tracks_for(sources, &items).await {
                Ok(tracks) => tracks,
                Err(e) => return ServerMessage::err(id, e.to_string()),
            };
            let cmd = match at {
                QueueAt::End => Command::EnqueueMany(tracks),
                QueueAt::Next => Command::PlayNext(tracks),
                QueueAt::Now => Command::Replace { tracks, start },
            };
            command(state, id, cmd).await
        }
        ClientMessage::Search {
            query,
            service,
            limit,
        } => {
            with_library(state, id, |library, sources| async move {
                let service = browse_service(&sources, service)?;
                let limit = limit.unwrap_or(SEARCH_LIMIT);
                let found = library.search(&sources, service, &query, limit).await?;
                Ok(ReplyData::Search(found))
            })
            .await
        }
        ClientMessage::Album { item } => {
            with_library(state, id, |library, sources| async move {
                Ok(ReplyData::Album(library.album(&sources, &item).await?))
            })
            .await
        }
        ClientMessage::Artist { item } => {
            with_library(state, id, |library, sources| async move {
                Ok(ReplyData::Artist(library.artist(&sources, &item).await?))
            })
            .await
        }
        ClientMessage::Radio { item } => {
            with_library(state, id, |library, sources| async move {
                let radio = library.radio(&sources, &item).await?;
                Ok(ReplyData::Tracks {
                    tracks: library.track_views(&sources, radio).await?,
                })
            })
            .await
        }
        ClientMessage::Similar { item } => {
            with_library(state, id, |library, sources| async move {
                Ok(ReplyData::Artists {
                    artists: library.similar(&sources, &item).await?,
                })
            })
            .await
        }
        ClientMessage::Import { service } => {
            with_library(state, id, |library, sources| async move {
                let service = browse_service(&sources, service)?;
                Ok(ReplyData::Imported(
                    library.import(&sources, service).await?,
                ))
            })
            .await
        }
        ClientMessage::Playlist { playlist } => {
            with_library(state, id, |library, sources| async move {
                Ok(ReplyData::Playlist(
                    library.playlist(&sources, playlist).await?,
                ))
            })
            .await
        }
        ClientMessage::PlaylistCreate { name, items } => {
            with_library(state, id, |library, sources| async move {
                let created = library.create_playlist(&sources, name, &items).await?;
                Ok(ReplyData::Playlist(created))
            })
            .await
        }
        ClientMessage::PlaylistRename { playlist, name } => {
            with_library(state, id, |library, _| async move {
                library.rename_playlist(playlist, name).await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
        ClientMessage::PlaylistDelete { playlist } => {
            with_library(state, id, |library, _| async move {
                library.delete_playlist(playlist).await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
        ClientMessage::PlaylistAdd {
            playlist,
            items,
            at,
        } => {
            with_library(state, id, |library, sources| async move {
                library.playlist_add(&sources, playlist, &items, at).await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
        ClientMessage::PlaylistRemove { playlist, index } => {
            with_library(state, id, |library, _| async move {
                library.playlist_remove(playlist, index).await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
        ClientMessage::PlaylistMove { playlist, from, to } => {
            with_library(state, id, |library, _| async move {
                library.playlist_move(playlist, from, to).await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
        ClientMessage::Mixes { service } => {
            with_library(state, id, |library, sources| async move {
                let service = browse_service(&sources, service)?;
                Ok(ReplyData::Mixes {
                    mixes: library.mixes(&sources, service).await?,
                })
            })
            .await
        }
        ClientMessage::Mix { service, mix } => {
            with_library(state, id, |library, sources| async move {
                let service = browse_service(&sources, service)?;
                let tracks = library.mix(&sources, service, &mix).await?;
                Ok(ReplyData::Tracks {
                    tracks: library.track_views(&sources, tracks).await?,
                })
            })
            .await
        }
        ClientMessage::Save { item } => {
            with_library(state, id, |library, sources| async move {
                library.save(&sources, &item).await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
        ClientMessage::Unsave { item } => {
            with_library(state, id, |library, sources| async move {
                library.unsave(&sources, &item).await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
        ClientMessage::Library {
            kind,
            query,
            limit,
            offset,
        } => {
            with_library(state, id, |library, sources| async move {
                let limit = limit.unwrap_or(LIBRARY_PAGE);
                let page = library.saved(&sources, kind, query, limit, offset).await?;
                Ok(ReplyData::Library(page))
            })
            .await
        }
        ClientMessage::Jump { index } => command(state, id, Command::Jump(index)).await,
        ClientMessage::Remove { index } => command(state, id, Command::Remove(index)).await,
        ClientMessage::Move { from, to } => command(state, id, Command::Move { from, to }).await,
        ClientMessage::Shuffle => command(state, id, Command::Shuffle).await,
        ClientMessage::Repeat { mode } => command(state, id, Command::SetRepeat(mode)).await,
        ClientMessage::Next => command(state, id, Command::Next).await,
        ClientMessage::Previous => command(state, id, Command::Previous).await,
        ClientMessage::Clear => command(state, id, Command::Clear).await,

        // --- connections: request/response ---
        ClientMessage::Services => {
            with_library(state, id, |_, sources| async move {
                let mut services = Vec::new();
                for connector in sources.connectors() {
                    services.push(ServiceView {
                        service: connector.service(),
                        methods: connector.methods(),
                        connections: connector.connections().await,
                    });
                }
                Ok(ReplyData::Services { services })
            })
            .await
        }
        ClientMessage::Connect { method } => {
            with_library(state, id, |_, sources| async move {
                let login = connector_for(&sources, &method)?.begin(&method).await?;
                Ok(ReplyData::Connecting { method, login })
            })
            .await
        }
        ClientMessage::ConnectComplete { method, redirect } => {
            with_library(state, id, |_, sources| async move {
                let status = connector_for(&sources, &method)?
                    .complete(&method, redirect)
                    .await?;
                Ok(ReplyData::Login { status })
            })
            .await
        }
        ClientMessage::Disconnect { method } => {
            with_library(state, id, |_, sources| async move {
                connector_for(&sources, &method)?
                    .disconnect(&method)
                    .await?;
                Ok(ReplyData::Ack)
            })
            .await
        }
    }
}

/// The library's track for a service + track id (for play/enqueue). Only id-based services are
/// addressable this way; local files are not named by id.
async fn track(
    state: &AppState,
    service: Service,
    track_id: &str,
) -> Result<canon_core::TrackRef, String> {
    let Some(binding) = SourceRef::by_id(service, track_id) else {
        return Err(format!("a {service} track has no id to name it by"));
    };
    let Some((library, sources)) = &state.library else {
        return Err("the library is unavailable".into());
    };
    library
        .track_for(sources, &binding)
        .await
        .map_err(|e| e.to_string())
}

async fn command(state: &AppState, id: Option<u64>, cmd: Command) -> ServerMessage {
    match state.control.dispatch(cmd).await {
        Ok(()) => ServerMessage::ack(id),
        Err(e) => ServerMessage::err(id, e.to_string()),
    }
}

/// Run `f` against the library and the sources it describes and browses with, mapping the result
/// into a reply. The handles are cheap clones, so `f`'s future owns what it uses.
async fn with_library<F, Fut>(state: &AppState, id: Option<u64>, f: F) -> ServerMessage
where
    F: FnOnce(Library, Sources) -> Fut,
    Fut: std::future::Future<Output = canon_core::Result<ReplyData>>,
{
    let Some((library, sources)) = &state.library else {
        return ServerMessage::err(id, "the library is unavailable");
    };
    match f(library.clone(), sources.clone()).await {
        Ok(data) => ServerMessage::ok(id, data),
        Err(e) => ServerMessage::err(id, e.to_string()),
    }
}

/// The connector that offers login method `method`.
fn connector_for(sources: &Sources, method: &str) -> canon_core::Result<Arc<dyn Connector>> {
    sources
        .connectors()
        .iter()
        .find(|connector| connector.methods().iter().any(|m| m.id == method))
        .cloned()
        .ok_or_else(|| canon_core::Error::NotFound(format!("no login method {method}")))
}

/// The service a browse goes to: the one asked for, else the first that can be browsed.
fn browse_service(sources: &Sources, asked: Option<Service>) -> canon_core::Result<Service> {
    asked.or_else(|| sources.default_catalog()).ok_or_else(|| {
        canon_core::Error::Unsupported(
            "nothing can be browsed: connect a service first (`services` lists the ways)".into(),
        )
    })
}

/// Encode and send one server frame. Server messages are plain serde types, so
/// serialization is infallible; a send error means the socket is gone.
async fn send(socket: &mut WebSocket, message: &ServerMessage) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).expect("server messages serialize");
    socket.send(Message::Text(text.into())).await
}
