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

/// What a service says about one of its tracks: enough for the library to find, or create, the
/// canon entity it is (yak canon-f7da). Everything but the title is best effort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTrack {
    /// The binding this describes.
    pub source: SourceRef,
    pub title: String,
    /// The credited artists, in order.
    pub artists: Vec<SourceArtist>,
    /// The release this binding is on. Services bind a track *as it appears on* an album.
    pub album: Option<SourceAlbum>,
    /// Disc and position on `album`, 1-based.
    pub disc: Option<u32>,
    pub position: Option<u32>,
    pub duration_ms: Option<u64>,
    /// The recording's ISRC: the key that finds the same recording on another service.
    pub isrc: Option<String>,
}

/// An artist as a service credits it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceArtist {
    /// The service's own id for the artist, if it has one.
    pub source: Option<SourceRef>,
    pub name: String,
}

/// A release as a service lists it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceAlbum {
    /// The service's own id for the album, if it has one.
    pub source: Option<SourceRef>,
    pub title: String,
    pub artists: Vec<SourceArtist>,
    pub release_date: Option<String>,
    /// UPC/EAN.
    pub barcode: Option<String>,
    pub artwork_url: Option<String>,
}

impl SourceTrack {
    /// The display metadata this implies.
    #[must_use]
    pub fn meta(&self) -> TrackMeta {
        TrackMeta {
            title: self.title.clone(),
            artists: self.artists.iter().map(|a| a.name.clone()).collect(),
            album: self.album.as_ref().map(|a| a.title.clone()),
            duration_ms: self.duration_ms,
            artwork_url: self.album.as_ref().and_then(|a| a.artwork_url.clone()),
        }
    }
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

impl Codec {
    /// The file extension that tells the demuxer what container to expect, where one
    /// helps: FLAC may be bare, and AAC/ALAC arrive in MP4.
    #[must_use]
    pub fn extension_hint(self) -> Option<&'static str> {
        match self {
            Codec::Flac => Some("flac"),
            Codec::Aac | Codec::Alac => Some("m4a"),
            _ => None,
        }
    }
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
