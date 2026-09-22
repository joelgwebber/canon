//! What the player plays, and the physical description of a resolved stream.

use serde::{Deserialize, Serialize};

use crate::{EntityId, SourceRef};

/// Display metadata for a track — feeds UIs and OS "now playing" integrations.
///
/// Durations are milliseconds on the wire (the control plane is WebSocket + JSON,
/// decided in yak canon-b46b), which keeps the JSON contract unambiguous for
/// hand-rolled TUI/native clients.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackMeta {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub artwork_url: Option<String>,
}

/// EBU R128 loudness info used by the ReplayGain DSP stage. Sourced from the service
/// (Tidal ships these on the stream) or computed for local files.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReplayGain {
    pub track_gain_db: f32,
    pub track_peak: f32,
    pub album_gain_db: Option<f32>,
    pub album_peak: Option<f32>,
}

/// What the player is told to play: a canon entity, its display metadata, and the
/// ordered set of source bindings it may resolve (first that works wins, subject to
/// policy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackRef {
    pub id: EntityId,
    pub meta: TrackMeta,
    pub sources: Vec<SourceRef>,
}

/// The codecs canon expects to decode (via Symphonia; fMP4 demux is the known risk
/// tracked on yak canon-c4c3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Flac,
    Aac,
    Alac,
    Vorbis,
    Mp3,
    Pcm,
    Other,
}

/// Physical description of a decoded stream, produced by a [`crate::Source`] at
/// resolve time. Internal (never serialized) — the audio pipeline uses it to size the
/// output format and drive the ReplayGain stage.
#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub codec: Codec,
    pub sample_rate: u32,
    pub bit_depth: Option<u8>,
    pub channels: u16,
    pub replaygain: Option<ReplayGain>,
}
