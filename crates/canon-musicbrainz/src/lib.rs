//! canon-musicbrainz — who the library's tracks and albums are, according to MusicBrainz
//! (yak canon-882a).
//!
//! The library joins services on ISRCs, but a recording is often released under several: a
//! reissue on another label, a remaster, a territory's own. A track known from Spotify under one
//! can sit on Tidal under another, and an ISRC lookup there finds nothing. MusicBrainz lists
//! every ISRC of a recording, so looking a track up by the ISRC it has teaches the library the
//! rest, and the next match onto a preferred service tries those too. The lookup also gives the
//! track its recording MBID, and albums their release and release-group MBIDs by barcode.
//!
//! [`Identifier`] does this in the background at MusicBrainz's pace, tracks some service was
//! found to lack first, and remembers what it has looked up so it asks again only after
//! [`RECHECK`].

mod client;

use std::sync::Arc;
use std::time::Duration;

use canon_core::{EntityId, Error, Result, SettingsStore};
use canon_library::{Identified, Library};
use uuid::Uuid;

pub use client::{CreditedArtist, Lookup, MusicBrainz, Recording, Release, same_barcode};

/// How long an entity looked up, found or not, is left before it is looked up again:
/// MusicBrainz grows every day.
pub const RECHECK: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// How many entities are picked up at a time.
const BATCH: usize = 50;
/// How long to wait when there is nothing to look up, or identifying is turned off.
const IDLE: Duration = Duration::from_secs(10 * 60);
/// How long to wait after MusicBrainz was unreachable or asked us to slow down.
const BACKOFF: Duration = Duration::from_secs(5 * 60);
/// How far apart a track's and a recording's lengths can be and still be the same recording,
/// when an ISRC names several.
const LENGTH_TOLERANCE_MS: u64 = 3_000;

/// Fills the library's MusicBrainz ids, and teaches it every ISRC of each recording.
pub struct Identifier {
    library: Library,
    lookup: Arc<dyn Lookup>,
}

/// What looking one track up found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackOutcome {
    /// The recording the track was told to be, if one was.
    pub recording: Option<Uuid>,
    /// What the library learned from it.
    pub identified: Identified,
}

impl Identifier {
    #[must_use]
    pub fn new(library: Library, lookup: Arc<dyn Lookup>) -> Self {
        Self { library, lookup }
    }

    /// Look track `id` up by each of its ISRCs, and record what was found.
    ///
    /// The recording is the one its ISRCs name, when they name one. When they name several,
    /// it is the one of those as long as the track (within a few seconds), if exactly one is;
    /// otherwise nothing is recorded but that the track was looked up. The track's artists take
    /// the recording's artists' MBIDs when the credits line up one for one, by name.
    ///
    /// # Errors
    /// MusicBrainz couldn't be asked ([`Error::Transient`] when it's worth trying again later),
    /// or the store failed. Nothing is recorded then.
    pub async fn identify_track(&self, id: EntityId) -> Result<TrackOutcome> {
        let (track, artist_names) = self
            .library
            .run(move |store| {
                let track = store
                    .track(id)?
                    .ok_or_else(|| Error::NotFound(format!("track {id}")))?;
                let names = track
                    .artists
                    .iter()
                    .map(|artist| Ok(store.artist(*artist)?.map(|a| a.name)))
                    .collect::<Result<Vec<_>>>()?;
                Ok((track, names))
            })
            .await?;

        let mut found: Vec<Recording> = Vec::new();
        for isrc in &track.isrcs {
            for recording in self.lookup.recordings_by_isrc(isrc).await? {
                if !found.iter().any(|known| known.mbid == recording.mbid) {
                    found.push(recording);
                }
            }
        }
        let chosen = choose(found, track.duration_ms);

        let recording = chosen.as_ref().map(|r| r.mbid);
        let isrcs = chosen.as_ref().map(|r| r.isrcs.clone()).unwrap_or_default();
        let artists: Vec<(EntityId, Uuid)> = match &chosen {
            Some(chosen) if credits_line_up(&artist_names, &chosen.artists) => track
                .artists
                .iter()
                .copied()
                .zip(chosen.artists.iter().map(|a| a.mbid))
                .collect(),
            _ => Vec::new(),
        };
        let identified = self
            .library
            .run(move |store| {
                store.atomically(|store| {
                    let identified = store.identify_track(id, recording, &isrcs)?;
                    if identified.duplicate.is_none() {
                        for (artist, mbid) in artists {
                            store.identify_artist(artist, mbid)?;
                        }
                    }
                    Ok(identified)
                })
            })
            .await?;
        Ok(TrackOutcome {
            recording,
            identified,
        })
    }

    /// Look album `id` up by its barcode, and record the release and its group when the barcode
    /// names one release, or several of which exactly one has the album's title.
    ///
    /// # Errors
    /// As [`Identifier::identify_track`].
    pub async fn identify_album(&self, id: EntityId) -> Result<Identified> {
        let album = self
            .library
            .run(move |store| {
                store
                    .album(id)?
                    .ok_or_else(|| Error::NotFound(format!("album {id}")))
            })
            .await?;
        let Some(barcode) = album.barcode.clone() else {
            return self
                .library
                .run(move |store| store.identify_album(id, None))
                .await;
        };
        let mut releases = self.lookup.releases_by_barcode(&barcode).await?;
        if releases.len() > 1 {
            releases.retain(|release| release.title.eq_ignore_ascii_case(album.title.trim()));
        }
        let release = match releases.as_slice() {
            [only] => Some((only.mbid, only.group)),
            _ => None,
        };
        self.library
            .run(move |store| store.identify_album(id, release))
            .await
    }

    /// Identify what the library holds, forever, at MusicBrainz's pace, while `settings` says to
    /// (`library.identify`). Meant to be spawned.
    pub async fn run(self, settings: Arc<dyn SettingsStore>) {
        let window = i64::try_from(RECHECK.as_millis()).unwrap_or(i64::MAX);
        loop {
            if !settings.get().library.identify {
                tokio::time::sleep(IDLE).await;
                continue;
            }
            let pending = self
                .library
                .run(move |store| {
                    Ok((
                        store.unidentified_tracks(BATCH, window)?,
                        store.unidentified_albums(BATCH, window)?,
                    ))
                })
                .await;
            let (tracks, albums) = match pending {
                Ok(pending) => pending,
                Err(e) => {
                    tracing::warn!("musicbrainz: reading the library: {e}");
                    tokio::time::sleep(BACKOFF).await;
                    continue;
                }
            };
            if tracks.is_empty() && albums.is_empty() {
                tokio::time::sleep(IDLE).await;
                continue;
            }
            if let Err(e) = self.batch(&tracks, &albums).await {
                tracing::warn!("musicbrainz: {e}; trying again in {BACKOFF:?}");
                tokio::time::sleep(BACKOFF).await;
            }
        }
    }

    /// Look up one batch. Stops at the first transient failure, which is returned: MusicBrainz
    /// is unreachable or wants a rest, and whatever wasn't reached is picked up next time. Any
    /// other failure is logged and the entity counted as looked up, so it isn't retried in a
    /// tight loop.
    async fn batch(&self, tracks: &[EntityId], albums: &[EntityId]) -> Result<()> {
        let (mut recordings, mut isrcs) = (0, 0);
        for &id in tracks {
            match self.identify_track(id).await {
                Ok(outcome) => {
                    recordings += usize::from(outcome.recording.is_some());
                    isrcs += outcome.identified.new_isrcs;
                    if let (Some(other), Some(mbid)) =
                        (outcome.identified.duplicate, outcome.recording)
                    {
                        tracing::info!(
                            "musicbrainz: tracks {id} and {other} are one recording ({mbid})"
                        );
                    }
                }
                Err(e @ Error::Transient(_)) => return Err(e),
                Err(e) => {
                    tracing::warn!("musicbrainz: track {id}: {e}");
                    self.library
                        .run(move |store| store.identify_track(id, None, &[]))
                        .await?;
                }
            }
        }
        for &id in albums {
            match self.identify_album(id).await {
                Ok(_) => {}
                Err(e @ Error::Transient(_)) => return Err(e),
                Err(e) => {
                    tracing::warn!("musicbrainz: album {id}: {e}");
                    self.library
                        .run(move |store| store.identify_album(id, None))
                        .await?;
                }
            }
        }
        tracing::info!(
            "musicbrainz: looked up {} tracks ({recordings} identified, {isrcs} new ISRCs) and {} \
             albums",
            tracks.len(),
            albums.len()
        );
        Ok(())
    }
}

/// The recording a track is, of those its ISRCs name.
fn choose(mut found: Vec<Recording>, duration_ms: Option<u64>) -> Option<Recording> {
    if found.len() > 1 {
        let want = duration_ms?;
        found.retain(|recording| {
            recording
                .length_ms
                .is_some_and(|length| length.abs_diff(want) <= LENGTH_TOLERANCE_MS)
        });
    }
    if found.len() == 1 { found.pop() } else { None }
}

/// Whether a track's artists and a recording's are the same people, in the same order.
fn credits_line_up(names: &[Option<String>], credited: &[CreditedArtist]) -> bool {
    names.len() == credited.len()
        && names.iter().zip(credited).all(|(name, artist)| {
            name.as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case(artist.name.trim()))
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use canon_core::{Service, SourceArtist, SourceRef, SourceTrack};

    use super::*;

    /// MusicBrainz, scripted.
    #[derive(Default)]
    struct Scripted {
        by_isrc: HashMap<String, Vec<Recording>>,
        by_barcode: HashMap<String, Vec<Release>>,
        down: bool,
        asked: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl Lookup for Scripted {
        async fn recordings_by_isrc(&self, isrc: &str) -> Result<Vec<Recording>> {
            self.asked.lock().unwrap().push(isrc.to_owned());
            if self.down {
                return Err(Error::Transient("503".into()));
            }
            Ok(self.by_isrc.get(isrc).cloned().unwrap_or_default())
        }

        async fn releases_by_barcode(&self, barcode: &str) -> Result<Vec<Release>> {
            self.asked.lock().unwrap().push(barcode.to_owned());
            Ok(self.by_barcode.get(barcode).cloned().unwrap_or_default())
        }
    }

    fn yes() -> CreditedArtist {
        CreditedArtist {
            mbid: Uuid::from_u128(1),
            name: "Yes".into(),
        }
    }

    fn khatru(mbid: u128, length_ms: u64) -> Recording {
        Recording {
            mbid: Uuid::from_u128(mbid),
            title: "Siberian Khatru".into(),
            length_ms: Some(length_ms),
            isrcs: vec!["USEE10301026".into(), "USEW20000054".into()],
            artists: vec![yes()],
        }
    }

    async fn library_with_khatru() -> (Library, EntityId) {
        let library = Library::new(canon_library::Store::open_in_memory().unwrap());
        let id = library
            .run(|store| {
                store.ingest_track(&SourceTrack {
                    source: SourceRef::Spotify {
                        id: "1nyLujWRDFnsuKkz1Iq387".into(),
                    },
                    title: "Siberian Khatru - 2003 Remaster".into(),
                    artists: vec![SourceArtist {
                        source: None,
                        name: "Yes".into(),
                    }],
                    album: None,
                    duration_ms: Some(534_720),
                    isrc: Some("usee10301026".into()),
                    disc: None,
                    position: None,
                })
            })
            .await
            .unwrap();
        library
            .run(move |store| store.mark_unmatched(id, Service::Tidal))
            .await
            .unwrap();
        (library, id)
    }

    #[tokio::test]
    async fn a_track_learns_its_recording_and_every_isrc_of_it() {
        let (library, id) = library_with_khatru().await;
        let lookup = Arc::new(Scripted {
            by_isrc: HashMap::from([("USEE10301026".into(), vec![khatru(7, 536_000)])]),
            ..Scripted::default()
        });
        let identifier = Identifier::new(library.clone(), lookup);

        let outcome = identifier.identify_track(id).await.unwrap();

        assert_eq!(outcome.recording, Some(Uuid::from_u128(7)));
        assert_eq!(outcome.identified.new_isrcs, 1);
        let (track, artist, unmatched) = library
            .run(move |store| {
                let track = store.track(id)?.unwrap();
                let artist = store.artist(track.artists[0])?.unwrap();
                let unmatched = store.unmatched_within(id, Service::Tidal, i64::MAX)?;
                Ok((track, artist, unmatched))
            })
            .await
            .unwrap();
        assert_eq!(track.mbid, Some(Uuid::from_u128(7)));
        assert_eq!(track.isrcs, ["USEE10301026", "USEW20000054"]);
        assert_eq!(artist.mbid, Some(Uuid::from_u128(1)));
        assert!(!unmatched, "Tidal gets asked again, under the new ISRC");
    }

    #[tokio::test]
    async fn an_isrc_naming_several_recordings_is_told_apart_by_length_or_not_at_all() {
        let (library, id) = library_with_khatru().await;
        let lookup = Arc::new(Scripted {
            by_isrc: HashMap::from([(
                "USEE10301026".into(),
                vec![khatru(7, 536_000), khatru(8, 620_000)],
            )]),
            ..Scripted::default()
        });
        let identifier = Identifier::new(library.clone(), lookup);
        assert_eq!(
            identifier.identify_track(id).await.unwrap().recording,
            Some(Uuid::from_u128(7))
        );

        assert_eq!(
            choose(vec![khatru(7, 536_000), khatru(8, 537_000)], Some(534_720)),
            None,
            "two plausible recordings: neither"
        );
        assert_eq!(
            choose(vec![khatru(7, 536_000), khatru(8, 537_000)], None),
            None
        );
    }

    #[tokio::test]
    async fn an_unknown_isrc_is_looked_up_once() {
        let (library, id) = library_with_khatru().await;
        let lookup = Arc::new(Scripted::default());
        let identifier = Identifier::new(library.clone(), lookup.clone());

        identifier.batch(&[id], &[]).await.unwrap();

        let pending = library
            .run(|store| store.unidentified_tracks(10, i64::MAX))
            .await
            .unwrap();
        assert!(pending.is_empty());
        assert_eq!(*lookup.asked.lock().unwrap(), ["USEE10301026"]);
    }

    #[tokio::test]
    async fn musicbrainz_being_down_leaves_the_track_for_later() {
        let (library, id) = library_with_khatru().await;
        let identifier = Identifier::new(
            library.clone(),
            Arc::new(Scripted {
                down: true,
                ..Scripted::default()
            }),
        );

        let failed = identifier.batch(&[id], &[]).await;

        assert!(matches!(failed, Err(Error::Transient(_))), "{failed:?}");
        let pending = library
            .run(|store| store.unidentified_tracks(10, i64::MAX))
            .await
            .unwrap();
        assert_eq!(pending, vec![id]);
    }

    #[tokio::test]
    async fn an_album_is_identified_by_barcode_and_title_among_several() {
        let library = Library::new(canon_library::Store::open_in_memory().unwrap());
        let id = library
            .run(|store| {
                store.add_album(&canon_library::Album {
                    title: "Close to the Edge".into(),
                    credit: "Yes".into(),
                    artists: Vec::new(),
                    release_date: None,
                    barcode: Some("00081227985393".into()),
                    mbid: None,
                    group_mbid: None,
                    artwork_url: None,
                })
            })
            .await
            .unwrap();
        let release = |mbid: u128, title: &str| Release {
            mbid: Uuid::from_u128(mbid),
            group: Some(Uuid::from_u128(99)),
            title: title.into(),
            barcode: Some("081227985393".into()),
        };
        let identifier = Identifier::new(
            library.clone(),
            Arc::new(Scripted {
                by_barcode: HashMap::from([(
                    "00081227985393".into(),
                    vec![release(5, "Close to the Edge"), release(6, "Other")],
                )]),
                ..Scripted::default()
            }),
        );

        identifier.identify_album(id).await.unwrap();

        let album = library
            .run(move |store| Ok(store.album(id)?.unwrap()))
            .await
            .unwrap();
        assert_eq!(album.mbid, Some(Uuid::from_u128(5)));
        assert_eq!(album.group_mbid, Some(Uuid::from_u128(99)));
    }
}
