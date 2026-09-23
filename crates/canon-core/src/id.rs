//! Identity: canon's stable handle, the services it federates, and how a track binds
//! to a concrete resolvable pointer on each service.
//!
//! Canon's *canonical* cross-service identity is the MusicBrainz recording MBID
//! (decided in yak canon-78ea); that richer entity model lives in `canon-library`.
//! canon-core only needs the lightweight handle the player and API pass around, plus
//! the per-service binding a `Source` knows how to resolve.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stable, service-agnostic handle for a library entity (a recording/track).
///
/// `canon-library` maps this to/from the MusicBrainz recording MBID (the canonical
/// external identity) and any number of per-service [`SourceRef`] bindings. Keeping
/// the handle canon-native is what lets a track survive losing or gaining any single
/// source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId(pub Uuid);

impl EntityId {
    /// A fresh id. Only the library mints ids for real entities (yak canon-f7da): an id made
    /// anywhere else names nothing the library knows, so the same track would become two.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// An upstream a source binding can point at. The local filesystem is modelled as a
/// service so "bring your own files" is a first-class source, not a special case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Local,
    Tidal,
    Spotify,
}

/// A concrete, resolvable pointer to one entity on one source.
///
/// A [`crate::TrackRef`] carries an ordered set of these; the player resolves them by
/// policy (local before streaming) so playback degrades gracefully when a source is
/// unavailable. The library also binds albums and artists with the same type; which kind of
/// entity a key names is the library's to record.
///
/// `Local` is provisional: a path is where a file is, not what it is, and the local index
/// (canon-5cb2) wants content-addressed identity instead. Don't build on the path being the
/// key; this variant's wire format will change.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "service", rename_all = "snake_case")]
pub enum SourceRef {
    Local { path: PathBuf },
    Tidal { id: String },
    Spotify { id: String },
}

impl Service {
    /// The stable wire name (matches the serde `snake_case` rename), used in JSON and
    /// in human-facing API error messages so both agree.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Service::Local => "local",
            Service::Tidal => "tidal",
            Service::Spotify => "spotify",
        }
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl SourceRef {
    /// The binding a service's own id names. The local filesystem has paths, not ids.
    #[must_use]
    pub fn by_id(service: Service, id: &str) -> Option<Self> {
        match service {
            Service::Tidal => Some(SourceRef::Tidal { id: id.to_string() }),
            Service::Spotify => Some(SourceRef::Spotify { id: id.to_string() }),
            Service::Local => None,
        }
    }

    #[must_use]
    pub fn service(&self) -> Service {
        match self {
            SourceRef::Local { .. } => Service::Local,
            SourceRef::Tidal { .. } => Service::Tidal,
            SourceRef::Spotify { .. } => Service::Spotify,
        }
    }
}

/// `tidal:55391786`, `local:/music/a.flac`: for messages and logs.
impl std::fmt::Display for SourceRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceRef::Tidal { id } | SourceRef::Spotify { id } => {
                write!(f, "{}:{id}", self.service())
            }
            SourceRef::Local { path } => write!(f, "local:{}", path.display()),
        }
    }
}

/// The streaming quality ladder, ascending. Maps onto Tidal's tiers; a [`crate::Source`]
/// clamps a request down to what the account/backend can actually serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Lossy, ~low bitrate (e.g. AAC 96k).
    Low,
    /// Lossy, high bitrate (e.g. AAC 320k).
    High,
    /// CD-quality FLAC (16-bit/44.1kHz).
    Lossless,
    /// Hi-res FLAC (24-bit, >44.1kHz).
    HiRes,
}
