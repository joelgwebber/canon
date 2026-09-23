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

use std::path::Path;
use std::sync::{Arc, Mutex};

use canon_core::{Error, Result, SourceRef, Sources, TrackRef};

pub use model::{Album, AlbumTrack, Artist, Binding, EntityKind, ItemRef, Provenance, Track};
pub use store::Store;

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
            match item {
                ItemRef::Entity { entity } => {
                    let entity = *entity;
                    tracks.extend(self.run(move |store| store.expand(entity)).await?);
                }
                ItemRef::Service { service, id, kind } => {
                    let binding = SourceRef::by_id(*service, id).ok_or_else(|| {
                        Error::Unsupported(format!("a {service} item has no id to name it by"))
                    })?;
                    match kind {
                        EntityKind::Track => tracks.push(self.track_for(sources, &binding).await?),
                        kind => {
                            let kind = *kind;
                            tracks.extend(
                                self.run(move |store| match store.bound(kind, &binding)? {
                                    Some(entity) => store.expand(entity),
                                    None => Err(Error::NotFound(format!(
                                        "{} {binding} is not in the library",
                                        kind.as_str()
                                    ))),
                                })
                                .await?,
                            );
                        }
                    }
                }
            }
        }
        Ok(tracks)
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use canon_core::{Quality, ResolvedStream, Service, Source, SourceTrack};

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
