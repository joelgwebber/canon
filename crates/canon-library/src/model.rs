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

impl ItemRef {
    /// What a link a user pasted points at, or `None` if it points at nothing canon can name.
    ///
    /// Tidal and Spotify both spell a link `…/<kind>/<id>`, with any number of segments in front
    /// that say nothing about the thing: a locale (`open.spotify.com/intl-de/track/…`), a
    /// `browse` prefix, or the album a track is being shown on
    /// (`listen.tidal.com/album/55391786/track/55391792`). The *last* `<kind>/<id>` pair wins,
    /// because that is what the page is showing. Spotify's `spotify:playlist:<id>` URI reads the
    /// same way once its colons are read as separators.
    ///
    /// This is the only way a playlist on a service can be named from outside: unlike a track or
    /// an album it is never ingested, so there is no canon id to paste instead.
    #[must_use]
    pub fn from_url(url: &str) -> Option<Self> {
        let text = url.trim();
        let (service, path) = match text.strip_prefix("spotify:") {
            Some(uri) => (Service::Spotify, uri.replace(':', "/")),
            None => {
                let after_scheme = text.split_once("://").map_or(text, |(_, rest)| rest);
                let (host, path) = after_scheme.split_once('/')?;
                let host = host.to_ascii_lowercase();
                let service = match host.trim_start_matches("www.") {
                    "tidal.com" | "listen.tidal.com" | "desktop.tidal.com" | "embed.tidal.com" => {
                        Service::Tidal
                    }
                    "spotify.com" | "open.spotify.com" | "play.spotify.com" => Service::Spotify,
                    _ => return None,
                };
                (service, path.to_owned())
            }
        };
        // Query and fragment are the service's business (`?si=…` tracks the share).
        let path = path.split(['?', '#']).next().unwrap_or_default();
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let mut found = None;
        for pair in segments.windows(2) {
            let (word, id) = (pair[0].to_ascii_lowercase(), pair[1].to_owned());
            let kind = match word.as_str() {
                "track" => EntityKind::Track,
                "album" => EntityKind::Album,
                "artist" => EntityKind::Artist,
                "playlist" => EntityKind::Playlist,
                "mix" => {
                    found = Some(ItemRef::Mix { service, mix: id });
                    continue;
                }
                _ => continue,
            };
            found = Some(ItemRef::Service { service, id, kind });
        }
        found
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tidal(id: &str, kind: EntityKind) -> Option<ItemRef> {
        Some(ItemRef::Service {
            service: Service::Tidal,
            id: id.into(),
            kind,
        })
    }

    fn spotify(id: &str, kind: EntityKind) -> Option<ItemRef> {
        Some(ItemRef::Service {
            service: Service::Spotify,
            id: id.into(),
            kind,
        })
    }

    /// A playlist is the one thing with no canon id to paste, so pasting its page is how it gets
    /// named (canon-d9e9). Share links carry `?si=`, and Spotify prefixes a locale.
    #[test]
    fn a_pasted_playlist_link_names_the_playlist() {
        let uuid = "ae995bd3-089f-4a50-bc78-be8d03d01ce3";
        assert_eq!(
            ItemRef::from_url(&format!("https://tidal.com/playlist/{uuid}")),
            tidal(uuid, EntityKind::Playlist)
        );
        assert_eq!(
            ItemRef::from_url(&format!("https://listen.tidal.com/playlist/{uuid}")),
            tidal(uuid, EntityKind::Playlist)
        );
        assert_eq!(
            ItemRef::from_url(&format!("http://www.tidal.com/browse/playlist/{uuid}")),
            tidal(uuid, EntityKind::Playlist),
            "the url tidal itself prints beside a search hit"
        );
        assert_eq!(
            ItemRef::from_url("https://open.spotify.com/playlist/37i9dQZF1DX?si=8f3c"),
            spotify("37i9dQZF1DX", EntityKind::Playlist)
        );
        assert_eq!(
            ItemRef::from_url("https://open.spotify.com/intl-de/playlist/37i9dQZF1DX"),
            spotify("37i9dQZF1DX", EntityKind::Playlist)
        );
        assert_eq!(
            ItemRef::from_url("spotify:playlist:37i9dQZF1DX"),
            spotify("37i9dQZF1DX", EntityKind::Playlist)
        );
    }

    #[test]
    fn a_pasted_link_names_tracks_albums_artists_and_mixes_too() {
        assert_eq!(
            ItemRef::from_url("https://tidal.com/browse/track/33348478"),
            tidal("33348478", EntityKind::Track)
        );
        assert_eq!(
            ItemRef::from_url("https://tidal.com/album/55391786"),
            tidal("55391786", EntityKind::Album)
        );
        assert_eq!(
            ItemRef::from_url("https://tidal.com/artist/9706"),
            tidal("9706", EntityKind::Artist)
        );
        assert_eq!(
            ItemRef::from_url("https://listen.tidal.com/mix/0026860c"),
            Some(ItemRef::Mix {
                service: Service::Tidal,
                mix: "0026860c".into(),
            })
        );
    }

    /// A track shown on its album is spelled with both in the path. The page is showing the
    /// track, so the track is what the link names.
    #[test]
    fn the_last_pair_of_a_link_wins() {
        assert_eq!(
            ItemRef::from_url("https://listen.tidal.com/album/55391786/track/55391792"),
            tidal("55391792", EntityKind::Track)
        );
    }

    #[test]
    fn a_link_to_nothing_canon_can_name_is_rejected() {
        for url in [
            "https://example.com/playlist/abc",
            "https://open.spotify.com/episode/abc",
            "https://tidal.com/playlist",
            "33348478",
            "",
        ] {
            assert_eq!(ItemRef::from_url(url), None, "{url}");
        }
    }
}
