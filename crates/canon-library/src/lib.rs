//! canon-library — the user's library as the source of truth.
//!
//! Where canon diverges hardest from tideway: canon owns identity and organization;
//! upstream services are interchangeable sources.
//!
//! Responsibilities (see yak canon-4185 and its children):
//! * **Entity model + persistence** (canon-78ea): canon-native tracks (recordings), albums
//!   (releases) and artists ([`model`]), with ISRCs and MusicBrainz ids as the cross-service
//!   identity, persisted in sqlite ([`Store`]).
//! * **Source bindings + resolution policy** (canon-880e): each entity holds any number of
//!   [`canon_core::SourceRef`] bindings with their provenance and confidence; ISRC (+ MBID) is
//!   the first-class join key, fuzzy match the persisted fallback.
//! * **Local index** (canon-5cb2): index local files by content + tags (not a required
//!   service id), embedding the canon id as a durable tag so it rebinds across moves.
//! * **Import/export** (canon-65f7): resolve imported playlists into canon entities
//!   (never a new upstream playlist), and export canon playlists back out.
//!
//! The store holds everything canon knows about, not only what the user has saved: browsing and
//! the queue create entities too, always through the library, so an id means the same thing
//! everywhere. Library membership is the separate `saved` set.

pub mod model;
mod schema;
mod store;
pub mod view;

use std::path::Path;
use std::sync::{Arc, Mutex};

use canon_core::{
    EntityId, Error, Result, Seed, Service, SourceRef, SourceTrack, Sources, TrackRef,
};

pub use model::{
    Album, AlbumTrack, Artist, Binding, EntityKind, ItemRef, Playlist, Provenance, Track,
};
pub use store::Store;
pub use view::{
    AlbumDetail, AlbumView, ArtistDetail, ArtistView, ImportReport, LibraryPage, ListedTrack,
    MixView, Named, PlaylistDetail, PlaylistView, SearchView, TrackView,
};

/// The library, shareable across tasks. Every operation runs on the blocking pool against the
/// one [`Store`], so a burst of writes can't stall the async runtime and each call sees the
/// previous one's effects.
#[derive(Clone)]
pub struct Library {
    store: Arc<Mutex<Store>>,
}

impl Library {
    /// Open (creating if need be) the library file at `path`.
    ///
    /// # Errors
    /// The directory or file can't be created or opened, or the schema can't be migrated.
    pub async fn open(path: &Path) -> Result<Self> {
        let path = path.to_path_buf();
        let store = tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Store::open(&path)
        })
        .await
        .map_err(|e| Error::Library(format!("opening the library: {e}")))??;
        Ok(Self::new(store))
    }

    /// A library over an existing store.
    #[must_use]
    pub fn new(store: Store) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }

    /// Run `f` against the store. Several calls inside one `f` see a consistent library: nothing
    /// else touches the store until it returns.
    ///
    /// # Errors
    /// Whatever `f` returns, or that the store is unusable after an earlier panic.
    pub async fn run<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Store) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            let mut store = store
                .lock()
                .map_err(|_| Error::Library("the store is poisoned by an earlier panic".into()))?;
            f(&mut store)
        })
        .await
        .map_err(|e| Error::Library(format!("library task: {e}")))?
    }

    /// The tracks `items` name, in order: a track is itself, an album its tracklist. This is what
    /// a queue edit plays.
    ///
    /// # Errors
    /// An item names nothing the library or its service knows, or names something that isn't a
    /// list of tracks (an artist).
    pub async fn tracks_for(&self, sources: &Sources, items: &[ItemRef]) -> Result<Vec<TrackRef>> {
        let mut tracks = Vec::new();
        for item in items {
            if let ItemRef::Mix { service, mix } = item {
                let ids = self.mix(sources, *service, mix).await?;
                tracks.extend(self.track_refs(ids).await?);
                continue;
            }
            let (known, _) = self.locate(sources, item).await?;
            match (item, known) {
                (
                    ItemRef::Service {
                        service,
                        id,
                        kind: EntityKind::Track,
                    },
                    _,
                ) => {
                    let binding = by_id(*service, id)?;
                    tracks.push(self.track_for(sources, &binding).await?);
                }
                // An album's tracklist is only whole once its listing has been fetched.
                (_, Some((_, EntityKind::Album)))
                | (
                    ItemRef::Service {
                        kind: EntityKind::Album,
                        ..
                    },
                    None,
                ) => {
                    let album = self.album(sources, item).await?.album.id;
                    tracks.extend(self.run(move |store| store.expand(album)).await?);
                }
                (_, Some((entity, _))) => {
                    tracks.extend(self.run(move |store| store.expand(entity)).await?);
                }
                (ItemRef::Entity { entity }, None) => {
                    return Err(Error::NotFound(format!("entity {entity}")));
                }
                // Expanded above; `locate` refuses a mix anyway.
                (ItemRef::Mix { .. }, None) => return Err(not_an_entity()),
                (ItemRef::Service { .. }, None) => {
                    return Err(Error::Unsupported(
                        "an artist is not a list of tracks: play an album, or their radio".into(),
                    ));
                }
            }
        }
        Ok(tracks)
    }

    /// Search `service`'s catalog. Every result is ingested, so each comes back with a canon id
    /// to play, save or open.
    ///
    /// # Errors
    /// The service can't be browsed, the search failed, or the store failed.
    pub async fn search(
        &self,
        sources: &Sources,
        service: Service,
        query: &str,
        limit: usize,
    ) -> Result<SearchView> {
        let found = sources.catalog(service)?.search(query, limit).await?;
        self.run(move |store| {
            store.atomically(|store| {
                let mut view = SearchView::default();
                for described in &found.tracks {
                    let id = store.ingest_track(described)?;
                    let on = described_album(store, described)?;
                    view.tracks.extend(store.track_view(id, on)?);
                }
                for described in &found.albums {
                    let id = store.ingest_album(described)?;
                    view.albums.extend(store.album_view(id)?);
                }
                for described in &found.artists {
                    let id = store.ingest_artist(described)?;
                    view.artists.extend(store.artist_view(id)?);
                }
                Ok(view)
            })
        })
        .await
    }

    /// An album and its whole tracklist. When the album is on a browsable service its listing is
    /// fetched and ingested first, so the tracklist is complete; if that fails, what the library
    /// already has is shown.
    ///
    /// # Errors
    /// `item` isn't an album, or names one the library doesn't have and the service can't list.
    pub async fn album(&self, sources: &Sources, item: &ItemRef) -> Result<AlbumDetail> {
        let (known, binding) = self.locate(sources, item).await?;
        if let Some((_, kind)) = known
            && kind != EntityKind::Album
        {
            return Err(Error::Unsupported(format!(
                "that is a {}, not an album",
                kind.as_str()
            )));
        }
        let fetched = match &binding {
            Some(binding) => match sources.catalog(binding.service())?.album(binding).await {
                Ok(listing) => Some(
                    self.run(move |store| store.ingest_album_listing(&listing))
                        .await?,
                ),
                Err(e) if known.is_some() => {
                    tracing::warn!("showing the library's copy of {binding}: {e}");
                    None
                }
                Err(e) => return Err(e),
            },
            None => None,
        };
        let id = fetched
            .or(known.map(|(id, _)| id))
            .ok_or_else(|| Error::NotFound("no such album".into()))?;
        self.run(move |store| {
            store
                .album_detail(id)?
                .ok_or_else(|| Error::NotFound(format!("album {id}")))
        })
        .await
    }

    /// An artist, their releases and top tracks. As for [`Library::album`], a browsable artist is
    /// fetched fresh; otherwise the library's albums credited to them are shown.
    ///
    /// # Errors
    /// `item` isn't an artist, or names one the library doesn't have and the service can't list.
    pub async fn artist(&self, sources: &Sources, item: &ItemRef) -> Result<ArtistDetail> {
        let (known, binding) = self.locate(sources, item).await?;
        if let Some((_, kind)) = known
            && kind != EntityKind::Artist
        {
            return Err(Error::Unsupported(format!(
                "that is a {}, not an artist",
                kind.as_str()
            )));
        }
        let listing = match &binding {
            Some(binding) => match sources.catalog(binding.service())?.artist(binding).await {
                Ok(listing) => Some(listing),
                Err(e) if known.is_some() => {
                    tracing::warn!("showing the library's copy of {binding}: {e}");
                    None
                }
                Err(e) => return Err(e),
            },
            None => None,
        };
        let known = known.map(|(id, _)| id);
        self.run(move |store| {
            store.atomically(|store| {
                let Some(listing) = listing else {
                    let id = known.ok_or_else(|| Error::NotFound("no such artist".into()))?;
                    let artist = store
                        .artist_view(id)?
                        .ok_or_else(|| Error::NotFound(format!("artist {id}")))?;
                    let mut albums = Vec::new();
                    for album in store.albums_by(id)? {
                        albums.extend(store.album_view(album)?);
                    }
                    return Ok(ArtistDetail {
                        artist,
                        albums,
                        top_tracks: Vec::new(),
                    });
                };
                let id = store.ingest_artist(&listing.artist)?;
                let mut albums = Vec::new();
                for described in &listing.albums {
                    let album = store.ingest_album(described)?;
                    albums.extend(store.album_view(album)?);
                }
                let mut top_tracks = Vec::new();
                for described in &listing.top_tracks {
                    let track = store.ingest_track(described)?;
                    let on = described_album(store, described)?;
                    top_tracks.extend(store.track_view(track, on)?);
                }
                Ok(ArtistDetail {
                    artist: store
                        .artist_view(id)?
                        .ok_or_else(|| Error::NotFound(format!("artist {id}")))?,
                    albums,
                    top_tracks,
                })
            })
        })
        .await
    }

    /// The service's radio for what `item` names (a track or an artist): tracks like it, as
    /// library entities, in the service's order.
    ///
    /// # Errors
    /// `item` is not a track or artist, it has no binding on a browsable service, or the call
    /// failed.
    pub async fn radio(&self, sources: &Sources, item: &ItemRef) -> Result<Vec<EntityId>> {
        let (seed, kind) = self.entity_for(sources, item).await?;
        let binding = self.browsable(sources, seed).await?;
        let seed = match kind {
            EntityKind::Track => Seed::Track(binding),
            EntityKind::Artist => Seed::Artist(binding),
            EntityKind::Album => {
                return Err(Error::Unsupported(
                    "radio starts from a track or an artist, not an album".into(),
                ));
            }
            EntityKind::Playlist => {
                return Err(Error::Unsupported(
                    "radio starts from a track or an artist, not a playlist".into(),
                ));
            }
        };
        let found = sources.catalog(seed_service(&seed))?.radio(&seed).await?;
        self.run(move |store| {
            store.atomically(|store| {
                found
                    .iter()
                    .map(|described| store.ingest_track(described))
                    .collect()
            })
        })
        .await
    }

    /// Artists the service says are like the one `item` names.
    ///
    /// # Errors
    /// `item` is not an artist on a browsable service, or the call failed.
    pub async fn similar(&self, sources: &Sources, item: &ItemRef) -> Result<Vec<ArtistView>> {
        let (artist, kind) = self.entity_for(sources, item).await?;
        if kind != EntityKind::Artist {
            return Err(Error::Unsupported(format!(
                "that is a {}, not an artist",
                kind.as_str()
            )));
        }
        let binding = self.browsable(sources, artist).await?;
        let found = sources
            .catalog(binding.service())?
            .similar_artists(&binding)
            .await?;
        self.run(move |store| {
            store.atomically(|store| {
                let mut views = Vec::with_capacity(found.len());
                for described in &found {
                    let id = store.ingest_artist(described)?;
                    views.extend(store.artist_view(id)?);
                }
                Ok(views)
            })
        })
        .await
    }

    /// Track views for `ids`, in order.
    ///
    /// # Errors
    /// The store failed.
    pub async fn track_views(&self, ids: Vec<EntityId>) -> Result<Vec<TrackView>> {
        self.run(move |store| {
            let mut views = Vec::with_capacity(ids.len());
            for id in ids {
                views.extend(store.track_view(id, None)?);
            }
            Ok(views)
        })
        .await
    }

    /// Playable tracks for `ids`, in order.
    ///
    /// # Errors
    /// The store failed.
    pub async fn track_refs(&self, ids: Vec<EntityId>) -> Result<Vec<TrackRef>> {
        self.run(move |store| {
            let mut refs = Vec::with_capacity(ids.len());
            for id in ids {
                refs.extend(store.track_ref(id)?);
            }
            Ok(refs)
        })
        .await
    }

    /// A binding of `entity` on a service that can be browsed.
    async fn browsable(&self, sources: &Sources, entity: EntityId) -> Result<SourceRef> {
        let bindings = self.run(move |store| store.bindings(entity)).await?;
        bindings
            .into_iter()
            .map(|binding| binding.source)
            .find(|source| sources.catalog(source.service()).is_ok())
            .ok_or_else(|| Error::Unsupported("it isn't on a service that can be browsed".into()))
    }

    /// Bring the user's favorites and playlists in from `service`. Favorites are saved as of when
    /// they were marked there; each service playlist becomes (or, imported again, updates) a canon
    /// playlist bound to it. One-way: nothing is written back to the service.
    ///
    /// # Errors
    /// The service can't be browsed or the calls failed; nothing is imported then.
    pub async fn import(&self, sources: &Sources, service: Service) -> Result<ImportReport> {
        let catalog = sources.catalog(service)?;
        let favorites = catalog.favorites().await?;
        let playlists = catalog.playlists().await?;
        self.run(move |store| {
            store.atomically(|store| {
                let mut report = ImportReport::default();
                for favorite in &favorites.tracks {
                    let id = store.ingest_track(&favorite.item)?;
                    store.save_at(id, favorite.added_ms)?;
                    report.tracks += 1;
                }
                for favorite in &favorites.albums {
                    let id = store.ingest_album(&favorite.item)?;
                    store.save_at(id, favorite.added_ms)?;
                    report.albums += 1;
                }
                for favorite in &favorites.artists {
                    let id = store.ingest_artist(&favorite.item)?;
                    store.save_at(id, favorite.added_ms)?;
                    report.artists += 1;
                }
                for playlist in &playlists {
                    let tracks = playlist
                        .tracks
                        .iter()
                        .map(|track| store.ingest_track(track))
                        .collect::<Result<Vec<_>>>()?;
                    match store.bound(EntityKind::Playlist, &playlist.source)? {
                        Some(id) => {
                            store.rename_playlist(id, &playlist.name)?;
                            store.edit_playlist(id, |list| {
                                *list = tracks;
                                Ok(())
                            })?;
                        }
                        None => {
                            let id = store.create_playlist(&playlist.name, &tracks)?;
                            store.bind(
                                EntityKind::Playlist,
                                id,
                                &Binding::direct(playlist.source.clone()),
                            )?;
                        }
                    }
                    report.playlists += 1;
                }
                Ok(report)
            })
        })
        .await
    }

    /// A new playlist called `name`, holding what `items` name (albums as their tracklists).
    ///
    /// # Errors
    /// An item can't be found, or the store failed.
    pub async fn create_playlist(
        &self,
        sources: &Sources,
        name: String,
        items: &[ItemRef],
    ) -> Result<PlaylistDetail> {
        let tracks = self.track_ids(sources, items).await?;
        self.run(move |store| {
            let id = store.create_playlist(&name, &tracks)?;
            store
                .playlist_detail(id)?
                .ok_or_else(|| Error::NotFound(format!("playlist {id}")))
        })
        .await
    }

    /// # Errors
    /// There is no such playlist, or the store failed.
    pub async fn playlist(&self, id: EntityId) -> Result<PlaylistDetail> {
        self.run(move |store| {
            store
                .playlist_detail(id)?
                .ok_or_else(|| Error::NotFound(format!("playlist {id}")))
        })
        .await
    }

    /// # Errors
    /// There is no such playlist, or the store failed.
    pub async fn rename_playlist(&self, id: EntityId, name: String) -> Result<()> {
        self.run(move |store| store.rename_playlist(id, &name))
            .await
    }

    /// # Errors
    /// There is no such playlist, or the store failed.
    pub async fn delete_playlist(&self, id: EntityId) -> Result<()> {
        if self.run(move |store| store.delete_playlist(id)).await? {
            Ok(())
        } else {
            Err(Error::NotFound(format!("playlist {id}")))
        }
    }

    /// Add what `items` name to playlist `id`, at position `at` (the end if `None`).
    ///
    /// # Errors
    /// No such playlist, `at` is past the end, an item can't be found, or the store failed.
    pub async fn playlist_add(
        &self,
        sources: &Sources,
        id: EntityId,
        items: &[ItemRef],
        at: Option<usize>,
    ) -> Result<()> {
        let tracks = self.track_ids(sources, items).await?;
        self.run(move |store| {
            store.edit_playlist(id, |list| {
                let at = at.unwrap_or(list.len());
                if at > list.len() {
                    return Err(Error::NotFound(format!("no position {at} in the playlist")));
                }
                list.splice(at..at, tracks);
                Ok(())
            })
        })
        .await
    }

    /// # Errors
    /// No such playlist or entry, or the store failed.
    pub async fn playlist_remove(&self, id: EntityId, index: usize) -> Result<()> {
        self.run(move |store| {
            store.edit_playlist(id, |list| {
                if index >= list.len() {
                    return Err(Error::NotFound(format!("no entry {index} in the playlist")));
                }
                list.remove(index);
                Ok(())
            })
        })
        .await
    }

    /// # Errors
    /// No such playlist or entry, or the store failed.
    pub async fn playlist_move(&self, id: EntityId, from: usize, to: usize) -> Result<()> {
        self.run(move |store| {
            store.edit_playlist(id, |list| {
                if from >= list.len() || to >= list.len() {
                    return Err(Error::NotFound(format!(
                        "no entry {} in the playlist",
                        from.max(to)
                    )));
                }
                let track = list.remove(from);
                list.insert(to, track);
                Ok(())
            })
        })
        .await
    }

    async fn track_ids(&self, sources: &Sources, items: &[ItemRef]) -> Result<Vec<EntityId>> {
        Ok(self
            .tracks_for(sources, items)
            .await?
            .into_iter()
            .map(|track| track.id)
            .collect())
    }

    /// The library entity `item` names, bringing it into the library first if it is on a service
    /// the library hasn't seen it from yet.
    ///
    /// # Errors
    /// Nothing by that id is in the library or on the service.
    pub async fn entity_for(
        &self,
        sources: &Sources,
        item: &ItemRef,
    ) -> Result<(EntityId, EntityKind)> {
        if let (Some(known), _) = self.locate(sources, item).await? {
            return Ok(known);
        }
        match item {
            ItemRef::Entity { entity } => Err(Error::NotFound(format!("entity {entity}"))),
            ItemRef::Mix { .. } => Err(not_an_entity()),
            ItemRef::Service { service, id, kind } => {
                let id = match kind {
                    EntityKind::Track => self.track_for(sources, &by_id(*service, id)?).await?.id,
                    EntityKind::Album => self.album(sources, item).await?.album.id,
                    EntityKind::Artist => self.artist(sources, item).await?.artist.id,
                    EntityKind::Playlist => {
                        return Err(Error::Unsupported(format!(
                            "playlists are canon's own; {service} ones can't be named here"
                        )));
                    }
                };
                Ok((id, *kind))
            }
        }
    }

    /// Put what `item` names in the user's library.
    ///
    /// # Errors
    /// As [`Library::entity_for`].
    pub async fn save(&self, sources: &Sources, item: &ItemRef) -> Result<EntityId> {
        let (id, _) = self.entity_for(sources, item).await?;
        self.run(move |store| store.save(id)).await?;
        Ok(id)
    }

    /// Take what `item` names out of the user's library. Returns whether it was in it.
    ///
    /// # Errors
    /// As [`Library::entity_for`].
    pub async fn unsave(&self, sources: &Sources, item: &ItemRef) -> Result<bool> {
        let (id, _) = self.entity_for(sources, item).await?;
        self.run(move |store| store.unsave(id)).await
    }

    /// A page of the saved library of one kind, filtered by `query` if given.
    ///
    /// # Errors
    /// The store failed.
    pub async fn saved(
        &self,
        kind: EntityKind,
        query: Option<String>,
        limit: usize,
        offset: usize,
    ) -> Result<LibraryPage> {
        self.run(move |store| {
            let (ids, total) = store.saved_page(kind, query.as_deref(), limit, offset)?;
            let mut page = LibraryPage {
                total,
                ..LibraryPage::default()
            };
            for id in ids {
                match kind {
                    EntityKind::Track => page.tracks.extend(store.track_view(id, None)?),
                    EntityKind::Album => page.albums.extend(store.album_view(id)?),
                    EntityKind::Artist => page.artists.extend(store.artist_view(id)?),
                    EntityKind::Playlist => page.playlists.extend(store.playlist_view(id)?),
                }
            }
            Ok(page)
        })
        .await
    }

    /// What `item` names: the library entity (and its kind) if the library has it, and a binding
    /// on a service that can be browsed for it, if there is one.
    async fn locate(
        &self,
        sources: &Sources,
        item: &ItemRef,
    ) -> Result<(Option<(EntityId, EntityKind)>, Option<SourceRef>)> {
        match item {
            ItemRef::Entity { entity } => {
                let entity = *entity;
                let (kind, bindings) = self
                    .run(move |store| Ok((store.kind_of(entity)?, store.bindings(entity)?)))
                    .await?;
                let browsable = bindings
                    .into_iter()
                    .map(|binding| binding.source)
                    .find(|source| sources.catalog(source.service()).is_ok());
                Ok((kind.map(|kind| (entity, kind)), browsable))
            }
            ItemRef::Service { service, id, kind } => {
                let binding = by_id(*service, id)?;
                let (kind, lookup) = (*kind, binding.clone());
                let known = self
                    .run(move |store| store.bound(kind, &lookup))
                    .await?
                    .map(|entity| (entity, kind));
                Ok((known, Some(binding)))
            }
            ItemRef::Mix { .. } => Err(not_an_entity()),
        }
    }

    /// The mixes `service` has made for the user.
    ///
    /// # Errors
    /// The service can't be browsed, or the call failed.
    pub async fn mixes(&self, sources: &Sources, service: Service) -> Result<Vec<MixView>> {
        let mixes = sources.catalog(service)?.mixes().await?;
        Ok(mixes
            .into_iter()
            .map(|mix| MixView {
                service,
                mix: mix.id,
                name: mix.name,
                description: mix.description,
            })
            .collect())
    }

    /// A mix's tracks, ingested, in order.
    ///
    /// # Errors
    /// The service can't be browsed, or the call failed.
    pub async fn mix(
        &self,
        sources: &Sources,
        service: Service,
        mix: &str,
    ) -> Result<Vec<EntityId>> {
        let found = sources.catalog(service)?.mix(mix).await?;
        self.run(move |store| {
            store.atomically(|store| {
                found
                    .iter()
                    .map(|described| store.ingest_track(described))
                    .collect()
            })
        })
        .await
    }

    /// The library's track for a service binding, ready to play: the one way a track id from
    /// outside (a client, a search result) becomes a canon entity (yak canon-f7da). A binding the
    /// library already has costs no network; a new one is described by its service and ingested.
    ///
    /// # Errors
    /// The service couldn't describe the binding, or the store failed.
    pub async fn track_for(&self, sources: &Sources, binding: &SourceRef) -> Result<TrackRef> {
        let known = binding.clone();
        let existing = self
            .run(move |store| match store.bound(EntityKind::Track, &known)? {
                Some(id) => store.track_ref(id),
                None => Ok(None),
            })
            .await?;
        if let Some(track) = existing {
            return Ok(track);
        }
        let described = sources.describe(binding).await?;
        self.run(move |store| {
            let id = store.ingest_track(&described)?;
            store
                .track_ref(id)?
                .ok_or_else(|| Error::Library(format!("track {id} vanished while ingesting")))
        })
        .await
    }
}

fn seed_service(seed: &Seed) -> Service {
    match seed {
        Seed::Track(source) | Seed::Artist(source) => source.service(),
    }
}

fn not_an_entity() -> Error {
    Error::Unsupported("a mix is a list of tracks, not a library entity".into())
}

/// A service's id as a binding.
fn by_id(service: Service, id: &str) -> Result<SourceRef> {
    SourceRef::by_id(service, id)
        .ok_or_else(|| Error::Unsupported(format!("a {service} item has no id to name it by")))
}

/// The library album a described track sits on, if the description names one it has.
fn described_album(store: &Store, described: &SourceTrack) -> Result<Option<EntityId>> {
    match described
        .album
        .as_ref()
        .and_then(|album| album.source.as_ref())
    {
        Some(source) => store.bound(EntityKind::Album, source),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use canon_core::{
        AlbumListing, ArtistListing, Catalog, Favorite, Favorites, Quality, ResolvedStream,
        SearchResults, Seed, Source, SourceAlbum, SourceArtist, SourceMix, SourcePlaylist,
    };

    use super::*;

    /// A Tidal stand-in that describes any id and counts how often it was asked.
    #[derive(Default)]
    struct Describer {
        asked: AtomicUsize,
    }

    #[async_trait]
    impl Source for Describer {
        fn service(&self) -> Service {
            Service::Tidal
        }

        async fn open(
            &self,
            _source: &SourceRef,
            _quality: Quality,
            _start: Duration,
        ) -> Result<ResolvedStream> {
            Err(Error::Unsupported("describe only".into()))
        }

        async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            Ok(SourceTrack {
                source: source.clone(),
                title: "Army of Me".into(),
                artists: Vec::new(),
                album: None,
                disc: None,
                position: None,
                duration_ms: Some(234_000),
                isrc: None,
            })
        }
    }

    fn tidal(id: &str) -> SourceRef {
        SourceRef::Tidal { id: id.into() }
    }

    fn floyd() -> SourceArtist {
        SourceArtist {
            source: Some(tidal("9706")),
            name: "Pink Floyd".into(),
        }
    }

    /// A track as it sits on Dark Side, abbreviated album and all, as Tidal's track lists give it.
    fn on_dsotm(id: &str, title: &str, position: u32) -> SourceTrack {
        SourceTrack {
            source: tidal(id),
            title: title.into(),
            artists: vec![floyd()],
            album: Some(SourceAlbum {
                source: Some(tidal("55391786")),
                title: "The Dark Side of the Moon".into(),
                ..SourceAlbum::default()
            }),
            disc: Some(1),
            position: Some(position),
            duration_ms: Some(200_000),
            isrc: None,
        }
    }

    /// A catalog that knows one album and one artist.
    struct FakeCatalog;

    #[async_trait]
    impl Catalog for FakeCatalog {
        fn service(&self) -> Service {
            Service::Tidal
        }
        async fn search(&self, _query: &str, _limit: usize) -> Result<SearchResults> {
            Ok(SearchResults {
                tracks: vec![on_dsotm("55391792", "Money", 6)],
                albums: vec![SourceAlbum {
                    source: Some(tidal("55391786")),
                    title: "The Dark Side of the Moon".into(),
                    ..SourceAlbum::default()
                }],
                artists: vec![floyd()],
            })
        }
        async fn album(&self, album: &SourceRef) -> Result<AlbumListing> {
            assert_eq!(album, &tidal("55391786"));
            Ok(AlbumListing {
                album: SourceAlbum {
                    source: Some(tidal("55391786")),
                    title: "The Dark Side of the Moon".into(),
                    artists: vec![floyd()],
                    release_date: Some("1973-03-01".into()),
                    ..SourceAlbum::default()
                },
                tracks: vec![
                    on_dsotm("55391787", "Speak to Me", 1),
                    on_dsotm("55391788", "Breathe", 2),
                    on_dsotm("55391792", "Money", 6),
                ],
            })
        }
        async fn artist(&self, artist: &SourceRef) -> Result<ArtistListing> {
            assert_eq!(artist, &tidal("9706"));
            Ok(ArtistListing {
                artist: floyd(),
                albums: vec![SourceAlbum {
                    source: Some(tidal("55391786")),
                    title: "The Dark Side of the Moon".into(),
                    ..SourceAlbum::default()
                }],
                top_tracks: vec![on_dsotm("55391792", "Money", 6)],
            })
        }
        async fn radio(&self, _seed: &Seed) -> Result<Vec<SourceTrack>> {
            Ok(vec![on_dsotm("55391790", "Time", 4)])
        }
        async fn similar_artists(&self, _artist: &SourceRef) -> Result<Vec<SourceArtist>> {
            Ok(Vec::new())
        }
        async fn favorites(&self) -> Result<Favorites> {
            Ok(Favorites {
                tracks: vec![Favorite {
                    item: on_dsotm("55391792", "Money", 6),
                    added_ms: Some(1_000),
                }],
                albums: Vec::new(),
                artists: vec![Favorite {
                    item: floyd(),
                    added_ms: Some(2_000),
                }],
            })
        }
        async fn mixes(&self) -> Result<Vec<SourceMix>> {
            Ok(vec![SourceMix {
                id: "0026860c".into(),
                name: "My Mix 1".into(),
                description: "Pink Floyd and more".into(),
            }])
        }
        async fn mix(&self, id: &str) -> Result<Vec<SourceTrack>> {
            assert_eq!(id, "0026860c");
            Ok(vec![
                on_dsotm("55391790", "Time", 4),
                on_dsotm("55391792", "Money", 6),
            ])
        }
        async fn playlists(&self) -> Result<Vec<SourcePlaylist>> {
            Ok(vec![SourcePlaylist {
                source: tidal("0f1e-playlist"),
                name: "Side two".into(),
                tracks: vec![
                    on_dsotm("55391792", "Money", 6),
                    on_dsotm("55391790", "Time", 4),
                ],
            }])
        }
    }

    #[tokio::test]
    async fn a_mix_is_listed_and_plays_as_its_tracks() {
        let (library, sources) = browsable();
        let mixes = library.mixes(&sources, Service::Tidal).await.unwrap();
        assert_eq!(mixes[0].name, "My Mix 1");
        let item: ItemRef =
            serde_json::from_str(r#"{"service": "tidal", "mix": "0026860c"}"#).unwrap();
        let tracks = library
            .tracks_for(&sources, std::slice::from_ref(&item))
            .await
            .unwrap();
        let titles: Vec<&str> = tracks.iter().map(|t| t.meta.title.as_str()).collect();
        assert_eq!(titles, ["Time", "Money"]);
        let error = library.save(&sources, &item).await.unwrap_err();
        assert!(
            error.to_string().contains("not a library entity"),
            "{error}"
        );
    }

    /// Importing brings favorites in as saved (keeping their dates) and playlists in as canon
    /// playlists; importing again updates rather than duplicates.
    #[tokio::test]
    async fn an_import_is_idempotent() {
        let (library, sources) = browsable();
        let report = library.import(&sources, Service::Tidal).await.unwrap();
        assert_eq!(
            report,
            ImportReport {
                tracks: 1,
                albums: 0,
                artists: 1,
                playlists: 1
            }
        );
        library.import(&sources, Service::Tidal).await.unwrap();

        let tracks = library.saved(EntityKind::Track, None, 10, 0).await.unwrap();
        assert_eq!(tracks.total, 1);
        assert_eq!(tracks.tracks[0].title, "Money");
        let artists = library
            .saved(EntityKind::Artist, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(artists.artists[0].name, "Pink Floyd");
        let playlists = library
            .saved(EntityKind::Playlist, None, 10, 0)
            .await
            .unwrap();
        assert_eq!(playlists.total, 1, "imported twice, still one");
        assert_eq!(playlists.playlists[0].name, "Side two");
        assert_eq!(playlists.playlists[0].track_count, 2);
    }

    fn browsable() -> (Library, Sources) {
        (
            Library::new(Store::open_in_memory().unwrap()),
            Sources::new().with_catalog(Arc::new(FakeCatalog)),
        )
    }

    /// Search results come back as library entities, and the same thing found twice is one.
    #[tokio::test]
    async fn search_results_are_library_entities() {
        let (library, sources) = browsable();
        let first = library
            .search(&sources, Service::Tidal, "money", 10)
            .await
            .unwrap();
        let again = library
            .search(&sources, Service::Tidal, "money", 10)
            .await
            .unwrap();
        assert_eq!(first, again);
        let money = &first.tracks[0];
        assert_eq!(money.title, "Money");
        assert_eq!(money.artists[0].name, "Pink Floyd");
        assert_eq!(money.artists[0].id, first.artists[0].id);
        assert_eq!(money.album.as_ref().unwrap().id, first.albums[0].id);
    }

    /// Opening an album fetches its listing: the tracklist pieced together from single tracks is
    /// replaced by the whole one, and the album gains the credits a track reply lacked.
    #[tokio::test]
    async fn opening_an_album_completes_it() {
        let (library, sources) = browsable();
        let found = library
            .search(&sources, Service::Tidal, "money", 10)
            .await
            .unwrap();
        let album = ItemRef::Entity {
            entity: found.albums[0].id,
        };
        let detail = library.album(&sources, &album).await.unwrap();
        assert_eq!(detail.album.credit, "Pink Floyd");
        assert_eq!(detail.album.release_date.as_deref(), Some("1973-03-01"));
        let listed: Vec<(u32, &str)> = detail
            .tracks
            .iter()
            .map(|t| (t.position, t.track.title.as_str()))
            .collect();
        assert_eq!(listed, [(1, "Speak to Me"), (2, "Breathe"), (6, "Money")]);
        assert_eq!(
            detail.tracks[2].track.id, found.tracks[0].id,
            "Money is Money"
        );

        let by_service = ItemRef::Service {
            service: Service::Tidal,
            id: "55391786".into(),
            kind: EntityKind::Album,
        };
        let queued = library.tracks_for(&sources, &[by_service]).await.unwrap();
        assert_eq!(queued.len(), 3, "an album queues as its tracklist");
    }

    #[tokio::test]
    async fn an_artist_page_lists_releases_and_top_tracks() {
        let (library, sources) = browsable();
        let artist = ItemRef::Service {
            service: Service::Tidal,
            id: "9706".into(),
            kind: EntityKind::Artist,
        };
        let detail = library.artist(&sources, &artist).await.unwrap();
        assert_eq!(detail.artist.name, "Pink Floyd");
        assert_eq!(detail.albums[0].title, "The Dark Side of the Moon");
        assert_eq!(detail.top_tracks[0].title, "Money");
        let error = library.tracks_for(&sources, &[artist]).await.unwrap_err();
        assert!(
            error.to_string().contains("not a list of tracks"),
            "{error}"
        );
        let error = library
            .album(
                &sources,
                &ItemRef::Entity {
                    entity: detail.artist.id,
                },
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("not an album"), "{error}");
    }

    #[tokio::test]
    async fn radio_comes_back_as_library_tracks() {
        let (library, sources) = browsable();
        let money = ItemRef::Service {
            service: Service::Tidal,
            id: "55391792".into(),
            kind: EntityKind::Track,
        };
        // The fake can't describe a lone track; bring Money in through its album first.
        let album = ItemRef::Service {
            service: Service::Tidal,
            id: "55391786".into(),
            kind: EntityKind::Album,
        };
        library.album(&sources, &album).await.unwrap();
        let radio = library.radio(&sources, &money).await.unwrap();
        let views = library.track_views(radio).await.unwrap();
        assert_eq!(views[0].title, "Time");
        assert_eq!(
            views[0].album.as_ref().unwrap().name,
            "The Dark Side of the Moon"
        );
        let error = library.radio(&sources, &album).await.unwrap_err();
        assert!(error.to_string().contains("not an album"), "{error}");
    }

    #[tokio::test]
    async fn a_playlist_is_an_ordered_list_that_plays_as_its_tracks() {
        let (library, sources) = browsable();
        let album = ItemRef::Service {
            service: Service::Tidal,
            id: "55391786".into(),
            kind: EntityKind::Album,
        };
        let detail = library.album(&sources, &album).await.unwrap();
        let [speak, breathe, money] = [0, 1, 2].map(|i| ItemRef::Entity {
            entity: detail.tracks[i].track.id,
        });

        let created = library
            .create_playlist(&sources, "Side one".into(), std::slice::from_ref(&money))
            .await
            .unwrap();
        let id = created.playlist.id;
        library
            .playlist_add(&sources, id, &[speak, breathe], Some(0))
            .await
            .unwrap();
        library.playlist_move(id, 2, 0).await.unwrap(); // Money to the top
        let titles = |p: &PlaylistDetail| -> Vec<String> {
            p.tracks.iter().map(|t| t.title.clone()).collect()
        };
        assert_eq!(
            titles(&library.playlist(id).await.unwrap()),
            ["Money", "Speak to Me", "Breathe"]
        );
        library.playlist_remove(id, 1).await.unwrap();
        library.rename_playlist(id, "Two".into()).await.unwrap();
        let shown = library.playlist(id).await.unwrap();
        assert_eq!(shown.playlist.name, "Two");
        assert_eq!(shown.playlist.track_count, 2);

        // It plays as its tracks, and lists as the user's.
        let queued = library
            .tracks_for(&sources, &[ItemRef::Entity { entity: id }])
            .await
            .unwrap();
        assert_eq!(queued.len(), 2);
        let listed = library
            .saved(EntityKind::Playlist, Some("tw".into()), 10, 0)
            .await
            .unwrap();
        assert_eq!(listed.playlists[0].id, id);

        assert!(library.playlist_remove(id, 5).await.is_err());
        library.delete_playlist(id).await.unwrap();
        assert!(library.playlist(id).await.is_err());
        assert!(library.delete_playlist(id).await.is_err());
        assert_eq!(
            library
                .track_views(vec![detail.tracks[2].track.id])
                .await
                .unwrap()[0]
                .title,
            "Money",
            "deleting a playlist leaves its tracks"
        );
    }

    #[tokio::test]
    async fn saving_by_service_id_brings_it_in_and_lists_it() {
        let (library, sources) = browsable();
        let album = ItemRef::Service {
            service: Service::Tidal,
            id: "55391786".into(),
            kind: EntityKind::Album,
        };
        let money = ItemRef::Service {
            service: Service::Tidal,
            id: "55391792".into(),
            kind: EntityKind::Track,
        };
        library.save(&sources, &album).await.unwrap();
        let saved_album = library.saved(EntityKind::Album, None, 10, 0).await.unwrap();
        assert_eq!(saved_album.total, 1);
        assert!(saved_album.albums[0].saved);
        assert_eq!(saved_album.albums[0].title, "The Dark Side of the Moon");

        // Money came in with the album, so saving it by id needs no describe.
        let id = library.save(&sources, &money).await.unwrap();
        let first = ItemRef::Entity { entity: id };
        let found = library
            .saved(EntityKind::Track, Some("mon".into()), 10, 0)
            .await
            .unwrap();
        assert_eq!(found.tracks[0].id, id);
        let found = library
            .saved(EntityKind::Track, Some("floyd".into()), 10, 0)
            .await
            .unwrap();
        assert_eq!(found.total, 1, "the credit matches too");
        let none = library
            .saved(EntityKind::Track, Some("100%".into()), 10, 0)
            .await
            .unwrap();
        assert_eq!(none.total, 0, "% is a character, not a wildcard");

        assert!(library.unsave(&sources, &first).await.unwrap());
        assert!(!library.unsave(&sources, &first).await.unwrap());
        assert_eq!(
            library
                .saved(EntityKind::Track, None, 10, 0)
                .await
                .unwrap()
                .total,
            0
        );
    }

    /// The tideway bug this closes: the same track enqueued twice was two entities.
    #[tokio::test]
    async fn one_binding_is_one_track_and_is_described_once() {
        let library = Library::new(Store::open_in_memory().unwrap());
        let describer = Arc::new(Describer::default());
        let sources = Sources::new().with(describer.clone());
        let army = SourceRef::Tidal {
            id: "33348478".into(),
        };

        let first = library.track_for(&sources, &army).await.unwrap();
        let second = library.track_for(&sources, &army).await.unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.meta.title, "Army of Me");
        assert_eq!(first.sources, vec![army]);
        assert_eq!(describer.asked.load(Ordering::SeqCst), 1);

        let spotify = SourceRef::Spotify { id: "x".into() };
        let error = library.track_for(&sources, &spotify).await.unwrap_err();
        assert!(error.to_string().contains("no spotify source"), "{error}");
    }

    #[tokio::test]
    async fn calls_run_in_order_against_one_store() {
        let library = Library::new(Store::open_in_memory().unwrap());
        let id = library
            .run(|store| {
                store.add_artist(&Artist {
                    name: "Björk".into(),
                    sort_name: None,
                    mbid: None,
                })
            })
            .await
            .unwrap();
        let name = library
            .run(move |store| Ok(store.artist(id)?.map(|a| a.name)))
            .await
            .unwrap();
        assert_eq!(name.as_deref(), Some("Björk"));
    }
}
