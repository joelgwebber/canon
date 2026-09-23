//! What clients are shown: library entities joined up for display, with the ids to act on.
//!
//! Every view carries canon ids (to play, save or open) and the entity's bindings (where it can
//! play from). They are read-only snapshots, built fresh per request.

use canon_core::{EntityId, SourceRef};
use serde::Serialize;

/// An entity named by id and display name, for a credit or an album reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Named {
    pub id: EntityId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
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
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlbumView {
    pub id: EntityId,
    pub title: String,
    pub credit: String,
    pub artists: Vec<Named>,
    pub release_date: Option<String>,
    pub artwork_url: Option<String>,
    pub saved: bool,
    pub sources: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArtistView {
    pub id: EntityId,
    pub name: String,
    pub saved: bool,
    pub sources: Vec<SourceRef>,
}

/// One entry of an album's tracklist.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListedTrack {
    pub disc: u32,
    pub position: u32,
    pub track: TrackView,
}

/// An album and its tracklist.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlbumDetail {
    pub album: AlbumView,
    pub tracks: Vec<ListedTrack>,
}

/// An artist, their releases, and their top tracks (when a service ranks them).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArtistDetail {
    pub artist: ArtistView,
    pub albums: Vec<AlbumView>,
    pub top_tracks: Vec<TrackView>,
}

/// A page of the user's saved library: one kind, newest first. `total` counts every match, not
/// just this page.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaylistView {
    pub id: EntityId,
    pub name: String,
    pub track_count: usize,
    /// Milliseconds since the Unix epoch.
    pub updated_at: i64,
}

/// A playlist and its tracks, in order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaylistDetail {
    pub playlist: PlaylistView,
    pub tracks: Vec<TrackView>,
}

/// What an import brought in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub tracks: usize,
    pub albums: usize,
    pub artists: usize,
    pub playlists: usize,
}

/// What a search found.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SearchView {
    pub tracks: Vec<TrackView>,
    pub albums: Vec<AlbumView>,
    pub artists: Vec<ArtistView>,
}
