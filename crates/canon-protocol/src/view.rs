//! What clients are shown: library entities joined up for display, with the ids to act on.
//!
//! Every view carries canon ids (to play, save or open) and the entity's bindings (where it can
//! play from). They are read-only snapshots, built fresh per request.

use canon_core::{EntityId, Service, SourceRef};
use serde::{Deserialize, Serialize};

/// An entity named by id and display name, for a credit or an album reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Named {
    pub id: EntityId,
    pub name: String,
}

/// Where an entity can be opened on a service's own site or app. The daemon builds these from the
/// entity's bindings (`canon_library::links`), so no client knows any service's URL scheme. A
/// local file has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub service: Service,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackView {
    pub id: EntityId,
    pub title: String,
    pub artists: Vec<Named>,
    /// The album this track is being shown on: the one listed, or the first it was placed on.
    pub album: Option<Named>,
    pub duration_ms: Option<u64>,
    pub artwork_url: Option<String>,
    /// Whether it is in the user's library.
    pub saved: bool,
    pub sources: Vec<SourceRef>,
    /// Where it opens on each service it is bound to, in the order of `sources`.
    #[serde(default)]
    pub links: Vec<Link>,
    /// The service it would play from now: the first binding, in the order playback tries them,
    /// on a service canon can stream. `None` means nothing bound is streamable, so pressing play
    /// fails unless queueing it finds a match first (by ISRC, onto a streaming service).
    pub plays_from: Option<Service>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumView {
    pub id: EntityId,
    pub title: String,
    pub credit: String,
    pub artists: Vec<Named>,
    pub release_date: Option<String>,
    pub artwork_url: Option<String>,
    pub saved: bool,
    pub sources: Vec<SourceRef>,
    /// Where it opens on each service it is bound to, in the order of `sources`.
    #[serde(default)]
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistView {
    pub id: EntityId,
    pub name: String,
    pub saved: bool,
    pub sources: Vec<SourceRef>,
    /// Where it opens on each service it is bound to, in the order of `sources`.
    #[serde(default)]
    pub links: Vec<Link>,
}

/// A track in full: the albums it is on and its credited artists, beside the track itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackDetail {
    pub track: TrackView,
    /// Every album the library has placed it on, in the order it learned of them.
    pub albums: Vec<AlbumView>,
    pub artists: Vec<ArtistView>,
}

/// One entry of an album's tracklist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListedTrack {
    pub disc: u32,
    pub position: u32,
    pub track: TrackView,
}

/// An album and its tracklist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AlbumDetail {
    pub album: AlbumView,
    pub tracks: Vec<ListedTrack>,
    /// Its credited artists, to follow from it (canon-a34e).
    #[serde(default)]
    pub artists: Vec<ArtistView>,
}

/// An artist, their releases, and their top tracks (when a service ranks them).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtistDetail {
    pub artist: ArtistView,
    pub albums: Vec<AlbumView>,
    pub top_tracks: Vec<TrackView>,
}

/// A page of the user's saved library: one kind, newest first. `total` counts every match, not
/// just this page.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryPage {
    pub total: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<TrackView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub albums: Vec<AlbumView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub artists: Vec<ArtistView>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub playlists: Vec<PlaylistView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistView {
    pub id: EntityId,
    pub name: String,
    pub track_count: usize,
    /// Milliseconds since the Unix epoch.
    pub updated_at: i64,
}

/// A playlist and its tracks, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistDetail {
    pub playlist: PlaylistView,
    pub tracks: Vec<TrackView>,
}

/// One recorded state of a playlist's track list. The newest is what the playlist holds now;
/// restore an older one by its `version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistVersion {
    pub version: u32,
    pub track_count: usize,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
}

/// A personal mix a service made for the user. Play it as `{"service", "mix"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MixView {
    pub service: Service,
    pub mix: String,
    pub name: String,
    pub description: String,
}

/// A playlist the user keeps on a service, without its tracks: enough to browse by. Play, copy
/// or merge it as the item `{"service", "id", "kind": "playlist"}` — it reads like a local
/// playlist, but canon never writes back to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServicePlaylistView {
    pub service: Service,
    /// The service's own id for it.
    pub id: String,
    pub name: String,
    pub track_count: usize,
}

/// What an import brought in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportReport {
    pub tracks: usize,
    pub albums: usize,
    pub artists: usize,
    pub playlists: usize,
}

/// What an export left on a service: the new playlist, and the tracks that didn't make it.
///
/// Export creates and never updates (yak canon-65f7), so `source` is always a playlist that did
/// not exist a moment ago. `skipped` names the tracks the service hasn't got — they are simply
/// absent from it, and nothing about the canon playlist changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportReport {
    pub service: Service,
    /// The playlist's name, the same on both sides.
    pub name: String,
    /// The service's binding for the playlist just created.
    pub source: SourceRef,
    /// What the description says, so the user can find this export again to delete it.
    pub description: String,
    pub exported: usize,
    /// Titles of the tracks the service has no copy of, in playlist order.
    pub skipped: Vec<String>,
}

/// What a search found. Tracks, albums and artists come back as library entities; playlists
/// don't, because a service's playlist is never saved as one (open or copy it instead).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchView {
    /// Whose results these are, on a partial reply of a search of every service; `None` on a
    /// search of one service and on the merged final reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<Service>,
    pub tracks: Vec<TrackView>,
    pub albums: Vec<AlbumView>,
    pub artists: Vec<ArtistView>,
    pub playlists: Vec<ServicePlaylistView>,
}

impl SearchView {
    /// How many results there are, of every kind.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tracks.len() + self.albums.len() + self.artists.len() + self.playlists.len()
    }

    /// Whether nothing was found.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fold `other`'s results in after these, leaving out what is already here: one rule for
    /// the server's merged reply and a client building up the partials. Tracks, albums and
    /// artists are the same result when they are the same library entity (ingest has already
    /// matched a recording found on two services); a playlist, which is never an entity, by
    /// its service and id. The merged view belongs to no one service.
    pub fn merge(&mut self, other: SearchView) {
        self.service = None;
        for track in other.tracks {
            if !self.tracks.iter().any(|have| have.id == track.id) {
                self.tracks.push(track);
            }
        }
        for album in other.albums {
            if !self.albums.iter().any(|have| have.id == album.id) {
                self.albums.push(album);
            }
        }
        for artist in other.artists {
            if !self.artists.iter().any(|have| have.id == artist.id) {
                self.artists.push(artist);
            }
        }
        for playlist in other.playlists {
            if !self
                .playlists
                .iter()
                .any(|have| have.service == playlist.service && have.id == playlist.id)
            {
                self.playlists.push(playlist);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: EntityId) -> TrackView {
        TrackView {
            id,
            title: "t".into(),
            artists: Vec::new(),
            album: None,
            duration_ms: None,
            artwork_url: None,
            saved: false,
            sources: Vec::new(),
            links: Vec::new(),
            plays_from: None,
        }
    }

    /// Merging keeps the first view's order and drops what is already there, by entity id for
    /// the library's kinds and by service and id for a playlist.
    #[test]
    fn merging_searches_leaves_out_what_is_already_there() {
        let (a, b) = (EntityId::new(), EntityId::new());
        let playlist = |service, id: &str| ServicePlaylistView {
            service,
            id: id.into(),
            name: "p".into(),
            track_count: 1,
        };
        let mut merged = SearchView {
            service: Some(Service::Spotify),
            tracks: vec![track(a)],
            playlists: vec![playlist(Service::Spotify, "1")],
            ..SearchView::default()
        };
        merged.merge(SearchView {
            service: Some(Service::Tidal),
            tracks: vec![track(a), track(b)],
            playlists: vec![
                playlist(Service::Tidal, "1"),
                playlist(Service::Spotify, "1"),
            ],
            ..SearchView::default()
        });
        assert_eq!(merged.service, None);
        let ids: Vec<_> = merged.tracks.iter().map(|t| t.id).collect();
        assert_eq!(ids, [a, b]);
        assert_eq!(
            merged.playlists.len(),
            2,
            "same id on two services is two playlists"
        );
        assert_eq!(merged.len(), 4);
    }
}
