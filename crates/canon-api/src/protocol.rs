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

use canon_core::{
    Account, DeviceCode, LoginStatus, PlayerSnapshot, Service, Settings, SinkInfo, TrackRef,
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
    Load {
        track: Box<TrackRef>,
    },
    /// Play a track named by service + track id (the daemon resolves and streams it),
    /// replacing the queue. Thin-client convenience over [`ClientMessage::Load`]. Note
    /// `track_id` is distinct from the envelope's correlation `id`.
    PlayTrack {
        service: Service,
        track_id: String,
    },
    /// Append a track to the server-owned queue (starts playback if idle).
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
    /// `queue`: the queue's entries, the current index, and the revision they are as of.
    Queue {
        revision: u64,
        index: usize,
        tracks: Vec<TrackRef>,
    },
}
