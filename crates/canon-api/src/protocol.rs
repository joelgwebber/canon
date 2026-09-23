//! The WebSocket+JSON wire protocol (yak canon-b46b, canon-9487).
//!
//! One JSON object per WebSocket text frame, in both directions. The transport choice
//! is WebSocket+JSON (canon-b46b) precisely so a hand-rolled TUI or native client can
//! speak it without a codegen step; the schema below is the whole contract.
//!
//! ## Client → server ([`ClientEnvelope`])
//!
//! Every frame is `{ "op": "...", ...fields }` plus an optional `"id"` the server
//! echoes back on the matching [`Reply`], so a client can correlate a response to the
//! request that caused it. Fire-and-forget transport verbs need no id.
//!
//! ## Server → client ([`ServerMessage`])
//!
//! * `hello` — once, on connect (protocol version).
//! * `snapshot` — the full authoritative [`PlayerSnapshot`], sent immediately on
//!   connect and again on every change. Each carries a monotonic `seq`, so a client
//!   reconciles by seq and interpolates position between snapshots using the reported
//!   `rate` — no high-frequency server poll (canon-9487). The snapshot is small and
//!   self-consistent, so canon ships full snapshots rather than field-level deltas;
//!   the seq contract leaves room to add true deltas later without a client change.
//! * `reply` — the response to one client request, correlated by `id`.

use canon_core::EntityId;
use canon_core::{
    Account, DeviceCode, LoginStatus, PlayerSnapshot, Repeat, Service, Settings, SinkInfo, TrackRef,
};
use canon_library::{
    AlbumDetail, ArtistDetail, ArtistView, EntityKind, ImportReport, ItemRef, LibraryPage, MixView,
    PlaylistDetail, SearchView, TrackView,
};
use serde::{Deserialize, Serialize};

/// The protocol version announced in `hello`. Bump on a breaking schema change.
pub const PROTOCOL_VERSION: u32 = 1;

/// A client frame: an optional correlation `id` plus the operation itself.
#[derive(Debug, Clone, Deserialize)]
pub struct ClientEnvelope {
    /// Echoed back on the resulting [`Reply`]. Optional for fire-and-forget verbs.
    #[serde(default)]
    pub id: Option<u64>,
    #[serde(flatten)]
    pub message: ClientMessage,
}

/// The operations a client can invoke, tagged by `op`.
///
/// Transport verbs translate straight into [`canon_core::Command`]; the `login_*` and
/// `account` verbs drive a [`canon_core::ServiceSession`]. Both faces bottom out in the
/// same core, so no playback or auth logic lives in a client.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ClientMessage {
    // --- transport (fire-and-forget into the player actor) ---
    Play,
    Pause,
    Stop,
    Seek {
        position_ms: u64,
    },
    SetVolume {
        volume: f32,
    },
    SetMuted {
        muted: bool,
    },
    SelectSink {
        sink: String,
    },
    /// List the selectable outputs (local plus discovered network renderers).
    ListSinks,
    /// The queue's contents. Its position and revision ride on every snapshot; fetch this when
    /// the revision moves.
    Queue,
    /// The current settings.
    Settings,
    /// Replace the settings whole (read, modify, write). Validated by the same schema that is
    /// persisted, so an unknown or retired key is an error reply, never a silent no-op.
    SetSettings {
        settings: Settings,
    },
    /// Play a track named by service + track id, replacing the queue. The library resolves the
    /// id to its canon entity (creating it on first sight), so the same id is always the same
    /// track; a client never names a track by a canon id it made up. Note `track_id` is distinct
    /// from the envelope's correlation `id`.
    PlayTrack {
        service: Service,
        track_id: String,
    },
    /// Append a track to the server-owned queue (starts playback if idle), resolved through the
    /// library as for `play_track`.
    Enqueue {
        service: Service,
        track_id: String,
    },
    /// Skip to the next queued track.
    Next,
    /// Skip to the previous queued track.
    Previous,
    /// Clear the queue and stop.
    Clear,
    /// Add what `items` name (tracks, albums) to the queue: at the end, right after the current
    /// entry, or in place of the whole queue, starting at the `start`th of the new tracks.
    QueueAdd {
        items: Vec<ItemRef>,
        #[serde(default)]
        at: QueueAt,
        #[serde(default)]
        start: usize,
    },
    // --- browsing (request/response; results are library entities, with ids to act on) ---
    /// Search a service's catalog (Tidal by default) for tracks, albums and artists.
    Search {
        query: String,
        #[serde(default)]
        service: Option<Service>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// An album and its tracklist.
    Album {
        item: ItemRef,
    },
    /// An artist, their releases, and their top tracks.
    Artist {
        item: ItemRef,
    },

    /// Tracks like a track or an artist: the service's radio for it.
    Radio {
        item: ItemRef,
    },
    /// Artists like an artist.
    Similar {
        item: ItemRef,
    },
    /// The mixes a service (Tidal by default) made for the user. Queue one as the item
    /// `{"service", "mix"}`.
    Mixes {
        #[serde(default)]
        service: Option<Service>,
    },
    /// A mix's tracks.
    Mix {
        #[serde(default)]
        service: Option<Service>,
        mix: String,
    },

    // --- the user's library ---
    /// Put what `item` names in the library (a service id the library hasn't seen is brought in).
    Save {
        item: ItemRef,
    },
    /// Take what `item` names out of the library.
    Unsave {
        item: ItemRef,
    },
    /// A page of the saved library of one kind (`track`, `album` or `artist`), newest first,
    /// optionally filtered by title, name or credit.
    Library {
        kind: EntityKind,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        offset: usize,
    },

    /// Bring the user's favorites (as saved) and playlists (as canon playlists) in from a service
    /// (Tidal by default). Safe to repeat: it updates rather than duplicates.
    Import {
        #[serde(default)]
        service: Option<Service>,
    },

    // --- playlists (canon's own; list them with `library` kind `playlist`) ---
    /// A playlist and its tracks.
    Playlist {
        playlist: EntityId,
    },
    /// A new playlist, optionally holding `items` (albums as their tracklists).
    PlaylistCreate {
        name: String,
        #[serde(default)]
        items: Vec<ItemRef>,
    },
    PlaylistRename {
        playlist: EntityId,
        name: String,
    },
    PlaylistDelete {
        playlist: EntityId,
    },
    /// Add `items` to a playlist at position `at` (0-based; the end if absent).
    PlaylistAdd {
        playlist: EntityId,
        items: Vec<ItemRef>,
        #[serde(default)]
        at: Option<usize>,
    },
    PlaylistRemove {
        playlist: EntityId,
        index: usize,
    },
    PlaylistMove {
        playlist: EntityId,
        from: usize,
        to: usize,
    },

    /// Start the queue entry at `index` (0-based).
    Jump {
        index: usize,
    },
    /// Remove the queue entry at `index`.
    Remove {
        index: usize,
    },
    /// Move the queue entry at `from` so it sits at `to`.
    Move {
        from: usize,
        to: usize,
    },
    /// Shuffle what comes after the current entry.
    Shuffle,
    /// Set what follows the end of a track: `off`, `all`, or `one`.
    Repeat {
        mode: Repeat,
    },

    // --- service session / auth (request/response) ---
    /// Begin device-code login for `service` (defaults to Tidal).
    LoginBegin {
        #[serde(default)]
        service: Option<Service>,
    },
    /// Poll the outstanding login once.
    LoginPoll {
        #[serde(default)]
        service: Option<Service>,
    },
    /// The authenticated "who am I" call — proof the token works end to end.
    Account {
        #[serde(default)]
        service: Option<Service>,
    },
}

/// Where `queue_add` puts its tracks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueAt {
    /// After everything already queued.
    #[default]
    End,
    /// Right after the current entry.
    Next,
    /// Instead of the queue, starting now.
    Now,
}

/// A server frame, tagged by `type`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Sent once on connect.
    Hello { protocol: u32 },
    /// The full authoritative snapshot (on connect and on every change).
    Snapshot { snapshot: PlayerSnapshot },
    /// The response to one client request.
    Reply {
        /// The request's `id`, if it carried one.
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<ReplyData>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

impl ServerMessage {
    /// A successful reply carrying data.
    #[must_use]
    pub fn ok(id: Option<u64>, result: ReplyData) -> Self {
        ServerMessage::Reply {
            id,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    /// A successful reply for a fire-and-forget command (no payload).
    #[must_use]
    pub fn ack(id: Option<u64>) -> Self {
        ServerMessage::Reply {
            id,
            ok: true,
            result: Some(ReplyData::Ack),
            error: None,
        }
    }

    /// A failed reply.
    #[must_use]
    pub fn err(id: Option<u64>, message: impl Into<String>) -> Self {
        ServerMessage::Reply {
            id,
            ok: false,
            result: None,
            error: Some(message.into()),
        }
    }
}

/// The payload of a successful [`ServerMessage::Reply`], tagged by `kind` so a client
/// can dispatch even without tracking which request an `id` belonged to.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplyData {
    /// A transport command was accepted.
    Ack,
    /// `login_begin`: the code + URL to show the user.
    DeviceCode(DeviceCode),
    /// `login_poll`: where the login stands.
    Login { status: LoginStatus },
    /// `account`: the authenticated identity.
    Account(Account),
    /// `list_sinks`: the selectable outputs, local first.
    Sinks { sinks: Vec<SinkInfo> },
    /// `settings`: the settings as they stand.
    Settings { settings: Settings },
    /// `search`: what matched, as library entities.
    Search(SearchView),
    /// `album`: the album and its tracklist.
    Album(AlbumDetail),
    /// `artist`: the artist, their releases and top tracks.
    Artist(ArtistDetail),
    /// `radio`: a list of tracks.
    Tracks { tracks: Vec<TrackView> },
    /// `similar`: a list of artists.
    Artists { artists: Vec<ArtistView> },
    /// `mixes`: the user's personal mixes.
    Mixes { mixes: Vec<MixView> },
    /// `playlist` and `playlist_create`: the playlist and its tracks.
    Playlist(PlaylistDetail),
    /// `import`: how much came in.
    Imported(ImportReport),
    /// `library`: a page of the saved library.
    Library(LibraryPage),
    /// `queue`: the queue's entries, the current index, and the revision they are as of.
    Queue {
        revision: u64,
        index: usize,
        tracks: Vec<TrackRef>,
    },
}
