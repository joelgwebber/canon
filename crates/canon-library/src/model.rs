//! The library's entities: what canon itself knows about music, independent of any service.
//!
//! The shape follows MusicBrainz where it matters for matching across services (yak canon-78ea):
//!
//! * A [`Track`] is a *recording*: the audio itself. ISRCs and the recording MBID identify it
//!   across services, and source bindings attach to it, because any binding of a recording plays
//!   the same audio. The album cut and the compilation cut of one recording are one track.
//! * An [`Album`] is a *release*: one edition, with its own tracklist and barcode. Editions of the
//!   same album share a release-group MBID. A recording appears on any number of albums, each at a
//!   disc and position.
//! * An [`Artist`] is credited, in order, on tracks and albums. The credit as displayed ("Simon &
//!   Garfunkel", "Björk feat. Skunk Anansie") is kept as written alongside the linked artists.
//!
//! Every entity has an [`EntityId`] from one id space, so a binding or a saved row names an entity
//! without saying which table it is in. The types here are the entities' data; the id is the
//! store's to mint (yak canon-f7da), so it is handed back on insert rather than carried inside.

use canon_core::{EntityId, Service, SourceRef};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Which kind of entity an id names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    #[default]
    Track,
    Album,
    Artist,
    /// The user's own ordered list of tracks. Canon's, never a service's.
    Playlist,
}

impl EntityKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            EntityKind::Track => "track",
            EntityKind::Album => "album",
            EntityKind::Artist => "artist",
            EntityKind::Playlist => "playlist",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "track" => Some(EntityKind::Track),
            "album" => Some(EntityKind::Album),
            "artist" => Some(EntityKind::Artist),
            "playlist" => Some(EntityKind::Playlist),
            _ => None,
        }
    }
}

/// Something a client asks for by reference: a library entity, or something on a service by the
/// service's own id (a track unless `kind` says otherwise). On the wire it is
/// `{"entity": "<uuid>"}` or `{"service": "tidal", "id": "55391786", "kind": "album"}`, or a
/// service's personal mix, `{"service": "tidal", "mix": "<mix id>"}`, which plays as its tracks
/// but is not itself an entity (mixes change daily).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ItemRef {
    Entity {
        entity: EntityId,
    },
    Mix {
        service: Service,
        mix: String,
    },
    Service {
        service: Service,
        id: String,
        #[serde(default)]
        kind: EntityKind,
    },
}

/// A performer or group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artist {
    pub name: String,
    /// "Beatles, The": how the artist files, when that differs from the name.
    pub sort_name: Option<String>,
    /// The MusicBrainz artist id.
    pub mbid: Option<Uuid>,
}

/// A release: one edition of an album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub title: String,
    /// The artist credit as displayed.
    pub credit: String,
    /// The credited artists, in order.
    pub artists: Vec<EntityId>,
    /// As precise as known: `1973`, `1973-03`, or `1973-03-01`.
    pub release_date: Option<String>,
    /// UPC/EAN: the one identifier the streaming services share for a release.
    pub barcode: Option<String>,
    /// The MusicBrainz release id (this edition).
    pub mbid: Option<Uuid>,
    /// The MusicBrainz release-group id (every edition of the album).
    pub group_mbid: Option<Uuid>,
    pub artwork_url: Option<String>,
}

/// A recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub title: String,
    /// The artist credit as displayed.
    pub credit: String,
    /// The credited artists, in order.
    pub artists: Vec<EntityId>,
    pub duration_ms: Option<u64>,
    /// A recording can carry several ISRCs (reissues under another label), and the join key
    /// across services is any of them.
    pub isrcs: Vec<String>,
    /// The MusicBrainz recording id.
    pub mbid: Option<Uuid>,
}

/// A playlist's own details (its tracks are listed separately).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub name: String,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    pub updated_at: i64,
}

/// Where a track sits on an album.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlbumTrack {
    pub disc: u32,
    pub position: u32,
    pub track: EntityId,
}

/// How a binding was established, from most to least certain. Kept with the binding so a
/// re-match never starts from nothing, and a weak match can be revisited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provenance {
    /// A person said so.
    Manual,
    /// The entity was created from this binding: it is what the binding describes.
    Direct,
    /// Matched on a shared MusicBrainz id.
    Mbid,
    /// Matched on a shared ISRC (tracks) or barcode (albums).
    Isrc,
    /// Matched on title, artist and duration.
    Fuzzy,
}

impl Provenance {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Provenance::Manual => "manual",
            Provenance::Direct => "direct",
            Provenance::Mbid => "mbid",
            Provenance::Isrc => "isrc",
            Provenance::Fuzzy => "fuzzy",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(Provenance::Manual),
            "direct" => Some(Provenance::Direct),
            "mbid" => Some(Provenance::Mbid),
            "isrc" => Some(Provenance::Isrc),
            "fuzzy" => Some(Provenance::Fuzzy),
            _ => None,
        }
    }
}

/// An entity's pointer into one service, with how much to trust it.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub source: SourceRef,
    pub provenance: Provenance,
    /// 0.0–1.0. Direct, manual and id-based matches are 1.0; fuzzy matches say how close.
    pub confidence: f32,
}

impl Binding {
    /// A binding the entity was created from.
    #[must_use]
    pub fn direct(source: SourceRef) -> Self {
        Self {
            source,
            provenance: Provenance::Direct,
            confidence: 1.0,
        }
    }
}
