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

use canon_core::{Error, Result};

pub use model::{Album, AlbumTrack, Artist, Binding, EntityKind, Provenance, Track};
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
