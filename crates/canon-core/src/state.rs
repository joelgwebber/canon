//! The authoritative playback state.
//!
//! This module encodes the single most important lesson from tideway (bug tide-2f85):
//! the emitted "now playing" desynced from reality because position/liveness lived in
//! the realtime audio callback while transport state lived elsewhere, and the output
//! device could change under both without anything re-emitting.
//!
//! Canon's fix, in types:
//! * The realtime callback owns **no** clock-of-record. It only counts the frames it
//!   emitted, and the engine reports that count to the player actor as an engine event
//!   (see [`crate::position`]).
//! * The actor publishes a seq-stamped [`PlayerSnapshot`] carrying the position those
//!   reports add up to. A stalled callback is therefore *visible* (reported frames stop
//!   advancing, and so does position) instead of an invisibly frozen emitter.
//!
//! The state *machine* (the actor that owns transitions and the emit loop) is built on
//! top of these types in yak canon-5afb.

use serde::{Deserialize, Serialize};

use crate::{PlayingFrom, SinkId, TrackRef};

/// The discrete transport state. Only the state actor mutates it, and every mutation
/// bumps the snapshot `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
    Error,
}

/// Where the server-owned play queue stands: its length, the index of the current entry, and a
/// revision that moves whenever its contents or position do. The entries themselves are fetched
/// separately (they would bloat a snapshot sent several times a second); `revision` tells a client
/// when its copy is stale.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueView {
    pub len: usize,
    pub index: usize,
    pub revision: u64,
    pub repeat: Repeat,
}

/// What follows the end of a track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Repeat {
    /// The next entry; the queue stops at its end.
    #[default]
    Off,
    /// The next entry, and the first again after the last.
    All,
    /// The same entry again. Skipping still moves on.
    One,
}

/// The single authoritative view of playback, stamped with a monotonic `seq`.
///
/// Clients reconcile by `seq`; a late joiner gets a full snapshot then deltas.
/// Position is reported with a `rate` (0.0 paused, 1.0 playing) so clients interpolate
/// between snapshots without a high-frequency server poll (yak canon-9487).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub seq: u64,
    pub state: PlaybackState,
    pub track: Option<TrackRef>,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub rate: f32,
    pub volume: f32,
    pub muted: bool,
    pub sink: Option<SinkId>,
    pub error: Option<String>,
    /// The queue this playback belongs to.
    #[serde(default)]
    pub queue: QueueView,
    /// Where the current track's audio comes from: which binding (so which service) opened, and
    /// what it turned out to be. `None` until the stream opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playing_from: Option<PlayingFrom>,
    /// The output that was lost from under playback, and why, until the listener next chooses
    /// an output. Not an `error`: playback carried on, just not where the listener left it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sink_lost: Option<SinkLoss>,
}

/// An output that went away without the listener choosing it (canon-60fa): a renderer that
/// dropped its connection and could not be reached again, was taken over, or stopped pulling
/// our stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinkLoss {
    /// The output that was lost (the one `sink` named until then).
    pub sink: SinkId,
    /// Why, in words a listener can act on.
    pub why: String,
}

impl PlayerSnapshot {
    /// The initial, nothing-loaded snapshot (`seq` 0).
    #[must_use]
    pub fn idle() -> Self {
        Self {
            seq: 0,
            state: PlaybackState::Idle,
            track: None,
            position_ms: 0,
            duration_ms: None,
            rate: 0.0,
            volume: 1.0,
            muted: false,
            sink: None,
            error: None,
            queue: QueueView::default(),
            playing_from: None,
            sink_lost: None,
        }
    }
}
