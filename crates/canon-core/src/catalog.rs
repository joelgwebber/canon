//! The `Catalog` seam: what a service can show you, as opposed to what it can play.
//!
//! A [`crate::Source`] opens bindings; a catalog finds them: search, an album's listing, an
//! artist's discography, and the service's own recommendations. Everything comes back as the
//! service describes it ([`SourceTrack`] and friends), never as library entities: the library
//! ingests these, so browsing never mints identity at the edge (yak canon-b989).

use async_trait::async_trait;

use crate::{Error, Result, Service, SourceAlbum, SourceArtist, SourceRef, SourceTrack};

/// What a search found, each list in the service's own relevance order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub tracks: Vec<SourceTrack>,
    pub albums: Vec<SourceAlbum>,
    pub artists: Vec<SourceArtist>,
}

/// An album and its whole tracklist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumListing {
    pub album: SourceAlbum,
    /// In disc and position order, each with its disc and position set.
    pub tracks: Vec<SourceTrack>,
}

/// An artist, their releases, and the tracks the service ranks highest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistListing {
    pub artist: SourceArtist,
    /// Albums, then EPs and singles, each newest first.
    pub albums: Vec<SourceAlbum>,
    pub top_tracks: Vec<SourceTrack>,
}

/// Something the user marked on a service, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Favorite<T> {
    pub item: T,
    /// Milliseconds since the Unix epoch, when the service says it was added.
    pub added_ms: Option<i64>,
}

/// Everything the user has marked as a favorite on a service, newest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Favorites {
    pub tracks: Vec<Favorite<SourceTrack>>,
    pub albums: Vec<Favorite<SourceAlbum>>,
    pub artists: Vec<Favorite<SourceArtist>>,
}

/// A playlist the user keeps on a service, with its tracks in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePlaylist {
    /// The service's own id for it.
    pub source: SourceRef,
    pub name: String,
    pub tracks: Vec<SourceTrack>,
}

/// A personal mix a service made for the user (Tidal's My Mix, Daily Discovery).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceMix {
    /// The service's id for the mix.
    pub id: String,
    pub name: String,
    pub description: String,
}

/// A service's browsable catalog.
#[async_trait]
pub trait Catalog: Send + Sync {
    /// Which service this catalog is.
    fn service(&self) -> Service;

    /// Tracks, albums and artists matching `query`, up to `limit` of each.
    async fn search(&self, query: &str, limit: usize) -> Result<SearchResults>;

    /// An album's details and full tracklist.
    async fn album(&self, album: &SourceRef) -> Result<AlbumListing>;

    /// An artist's details, releases and top tracks.
    async fn artist(&self, artist: &SourceRef) -> Result<ArtistListing>;

    /// Tracks like this one: the service's radio for a track, or for an artist.
    async fn radio(&self, seed: &Seed) -> Result<Vec<SourceTrack>>;

    /// Artists like this one.
    async fn similar_artists(&self, artist: &SourceRef) -> Result<Vec<SourceArtist>>;

    /// The signed-in user's favorites.
    async fn favorites(&self) -> Result<Favorites>;

    /// The signed-in user's own playlists.
    async fn playlists(&self) -> Result<Vec<SourcePlaylist>>;

    /// The mixes the service has made for the signed-in user.
    async fn mixes(&self) -> Result<Vec<SourceMix>>;

    /// A mix's tracks, in order.
    async fn mix(&self, id: &str) -> Result<Vec<SourceTrack>>;

    /// The service's tracks carrying `isrc`: one recording, often on several releases (the
    /// album, a compilation, a box set). Each returned track's `isrc` equals `isrc`. A service
    /// may leave out copies it can't stream, since the point of a match is to play it. Empty
    /// when the service has no such recording.
    ///
    /// # Errors
    /// The service can't look tracks up by ISRC (the default), or the call failed.
    async fn tracks_by_isrc(&self, isrc: &str) -> Result<Vec<SourceTrack>> {
        Err(Error::Unsupported(format!(
            "{} can't look up a track by ISRC ({isrc})",
            self.service()
        )))
    }
}

/// What a radio station is seeded with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seed {
    Track(SourceRef),
    Artist(SourceRef),
}
