//! The `Source` seam: turning a per-service [`SourceRef`] into decodable bytes.
//!
//! One implementation per service lives in its own crate (`canon-tidal` first, a
//! local-files source, and — proven feasible in yak canon-1175 — Spotify later). The
//! trait is intentionally ignorant of *how* bytes are fetched: the Tidal impl hides
//! DASH manifest resolution and transparent re-resolution of expired segment URLs
//! behind a plain reader (yak canon-e99d).
//!
//! [`Sources`] is the registry the daemon plays through: it holds one source per service
//! and resolves a [`TrackRef`]'s bindings by policy, so nothing above it names a service.

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::{
    Catalog, Error, Quality, Result, Service, SourceRef, SourceTrack, StreamInfo, TrackMeta,
    TrackRef,
};

/// A byte input the decode stage (`canon-audio`, Symphonia) can consume.
///
/// Blanket-implemented for anything that is `Read + Seek + Send + Sync`, so both a
/// plain `File` and the Tidal segment reader satisfy it. `Sync` is required because
/// Symphonia's `MediaSource` (the decoder boundary a `Box<dyn MediaInput>` is adapted
/// into) is `Read + Seek + Send + Sync`; the fMP4 decode spike confirmed the bound is
/// necessary and cheap to meet.
pub trait MediaInput: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> MediaInput for T {}

/// An opened, playable stream: the bytes, their physical description, and where in the
/// track they begin.
pub struct ResolvedStream {
    pub input: Box<dyn MediaInput>,
    pub info: StreamInfo,
    /// Where the bytes start, in track time. A source may start a little before the
    /// position asked for (Tidal starts at the segment covering it); playback reports
    /// position from here, so the clock stays true.
    pub start_ms: u64,
}

/// A music service (or the local filesystem) that opens bindings as streams and
/// answers metadata queries.
///
/// `async_trait` keeps this object-safe, so [`Sources`] can hold `Arc<dyn Source>`.
#[async_trait]
pub trait Source: Send + Sync {
    /// Which service this source serves.
    fn service(&self) -> Service;

    /// Open a binding as a stream starting at (or just before) `start`, at up to the
    /// requested quality (clamped to what the account/backend can serve).
    async fn open(
        &self,
        source: &SourceRef,
        quality: Quality,
        start: Duration,
    ) -> Result<ResolvedStream>;

    /// What the service knows about a track binding: what the library builds or matches its
    /// entity from, and where display metadata comes from.
    async fn describe(&self, source: &SourceRef) -> Result<SourceTrack>;
}

/// Every registered source and catalog, keyed by service, and the policy for choosing among a
/// track's bindings.
#[derive(Default, Clone)]
pub struct Sources {
    by_service: HashMap<Service, Arc<dyn Source>>,
    catalogs: HashMap<Service, Arc<dyn Catalog>>,
}

impl Sources {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `source` for its service, replacing any earlier one.
    #[must_use]
    pub fn with(mut self, source: Arc<dyn Source>) -> Self {
        self.by_service.insert(source.service(), source);
        self
    }

    /// Register `catalog` for its service, replacing any earlier one.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Arc<dyn Catalog>) -> Self {
        self.catalogs.insert(catalog.service(), catalog);
        self
    }

    /// The catalog for `service`.
    ///
    /// # Errors
    /// There is none: the service can't be browsed (or isn't set up).
    pub fn catalog(&self, service: Service) -> Result<&Arc<dyn Catalog>> {
        self.catalogs
            .get(&service)
            .ok_or_else(|| Error::Unsupported(format!("{service} can't be browsed")))
    }

    /// Open the first of `track`'s bindings that will open, in policy order.
    ///
    /// # Errors
    /// Every candidate's failure, or that the track has no binding any registered
    /// source can play.
    pub async fn open(
        &self,
        track: &TrackRef,
        quality: Quality,
        start: Duration,
    ) -> Result<ResolvedStream> {
        let mut failures = Vec::new();
        for (binding, source) in self.candidates(track) {
            match source.open(binding, quality, start).await {
                Ok(stream) => return Ok(stream),
                Err(e) => failures.push((binding.service(), e)),
            }
        }
        Err(Self::unplayable(track, failures))
    }

    /// Display metadata from the first binding in policy order that can describe it.
    ///
    /// # Errors
    /// As [`Sources::open`].
    pub async fn track_meta(&self, track: &TrackRef) -> Result<TrackMeta> {
        let mut failures = Vec::new();
        for (binding, source) in self.candidates(track) {
            match source.describe(binding).await {
                Ok(described) => return Ok(described.meta()),
                Err(e) => failures.push((binding.service(), e)),
            }
        }
        Err(Self::unplayable(track, failures))
    }

    /// What the service behind `binding` says about it.
    ///
    /// # Errors
    /// No source is registered for the binding's service, or it failed to describe it.
    pub async fn describe(&self, binding: &SourceRef) -> Result<SourceTrack> {
        let service = binding.service();
        let source = self
            .by_service
            .get(&service)
            .ok_or_else(|| Error::Source(format!("no {service} source is available")))?;
        source.describe(binding).await
    }

    /// `track`'s bindings that have a registered source, in the order to try them: local
    /// files before any streaming service (they need no network and no account), and
    /// otherwise in the order the track lists them.
    fn candidates<'a>(
        &'a self,
        track: &'a TrackRef,
    ) -> impl Iterator<Item = (&'a SourceRef, &'a Arc<dyn Source>)> {
        let mut bindings: Vec<&SourceRef> = track.sources.iter().collect();
        // Stable, so the track's own order breaks ties.
        bindings.sort_by_key(|binding| binding.service() != Service::Local);
        bindings.into_iter().filter_map(|binding| {
            self.by_service
                .get(&binding.service())
                .map(|source| (binding, source))
        })
    }

    /// Why nothing played. One failure is returned as it was, so its kind (an expired login,
    /// say) survives; several are summarised together.
    fn unplayable(track: &TrackRef, mut failures: Vec<(Service, Error)>) -> Error {
        if failures.len() == 1 {
            return failures.remove(0).1;
        }
        if failures.is_empty() {
            let services: Vec<&str> = track.sources.iter().map(|s| s.service().as_str()).collect();
            Error::Source(format!(
                "no source can play this track (bindings: {})",
                if services.is_empty() {
                    "none".to_string()
                } else {
                    services.join(", ")
                }
            ))
        } else {
            let each: Vec<String> = failures
                .iter()
                .map(|(service, e)| format!("{service}: {e}"))
                .collect();
            Error::Source(each.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::{Codec, EntityId};

    /// A source that records what it was asked to open, and fails if told to.
    struct Fake {
        service: Service,
        fails: bool,
        opened: Mutex<Vec<(SourceRef, Duration)>>,
    }

    impl Fake {
        fn new(service: Service, fails: bool) -> Arc<Self> {
            Arc::new(Self {
                service,
                fails,
                opened: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl Source for Fake {
        fn service(&self) -> Service {
            self.service
        }

        async fn open(
            &self,
            source: &SourceRef,
            _quality: Quality,
            start: Duration,
        ) -> Result<ResolvedStream> {
            self.opened.lock().unwrap().push((source.clone(), start));
            if self.fails {
                return Err(Error::Source("offline".into()));
            }
            Ok(ResolvedStream {
                input: Box::new(std::io::Cursor::new(Vec::new())),
                info: StreamInfo {
                    codec: Codec::Flac,
                    sample_rate: 44_100,
                    bit_depth: Some(16),
                    channels: 2,
                    replaygain: None,
                },
                #[allow(clippy::cast_possible_truncation)]
                start_ms: start.as_millis() as u64,
            })
        }

        async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
            if self.fails {
                return Err(Error::Source("offline".into()));
            }
            Ok(SourceTrack {
                source: source.clone(),
                title: format!("from {}", self.service),
                artists: Vec::new(),
                album: None,
                disc: None,
                position: None,
                duration_ms: None,
                isrc: None,
            })
        }
    }

    fn track(sources: Vec<SourceRef>) -> TrackRef {
        TrackRef {
            id: EntityId::new(),
            meta: TrackMeta::default(),
            sources,
        }
    }

    fn tidal(id: &str) -> SourceRef {
        SourceRef::Tidal { id: id.into() }
    }

    fn local(path: &str) -> SourceRef {
        SourceRef::Local {
            path: PathBuf::from(path),
        }
    }

    #[tokio::test]
    async fn a_local_file_is_preferred_over_streaming_whatever_the_order() {
        let tidal_source = Fake::new(Service::Tidal, false);
        let local_source = Fake::new(Service::Local, false);
        let sources = Sources::new()
            .with(tidal_source.clone())
            .with(local_source.clone());

        let start = Duration::from_secs(30);
        let stream = sources
            .open(
                &track(vec![tidal("1"), local("/a.flac")]),
                Quality::Lossless,
                start,
            )
            .await
            .unwrap();
        assert_eq!(stream.start_ms, 30_000);
        assert_eq!(
            *local_source.opened.lock().unwrap(),
            vec![(local("/a.flac"), start)]
        );
        assert!(tidal_source.opened.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failing_binding_falls_through_to_the_next() {
        let local_source = Fake::new(Service::Local, true);
        let tidal_source = Fake::new(Service::Tidal, false);
        let sources = Sources::new()
            .with(local_source.clone())
            .with(tidal_source.clone());

        let track = track(vec![local("/gone.flac"), tidal("1")]);
        sources
            .open(&track, Quality::Lossless, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(local_source.opened.lock().unwrap().len(), 1);
        assert_eq!(tidal_source.opened.lock().unwrap().len(), 1);
        assert_eq!(
            sources.track_meta(&track).await.unwrap().title,
            "from tidal"
        );
    }

    #[tokio::test]
    async fn every_failure_is_reported() {
        let sources = Sources::new()
            .with(Fake::new(Service::Local, true))
            .with(Fake::new(Service::Tidal, true));
        let track = track(vec![tidal("1"), local("/gone.flac")]);
        let error = sources
            .open(&track, Quality::Lossless, Duration::ZERO)
            .await
            .err()
            .expect("unplayable");
        assert_eq!(
            error.to_string(),
            "source: local: source: offline; tidal: source: offline"
        );

        let sources = Sources::new().with(Fake::new(Service::Tidal, true));
        let error = sources
            .open(&track, Quality::Lossless, Duration::ZERO)
            .await
            .err()
            .expect("unplayable");
        assert_eq!(
            error.to_string(),
            "source: offline",
            "one failure passes through"
        );
    }

    #[tokio::test]
    async fn a_binding_with_no_registered_source_is_skipped() {
        let sources = Sources::new().with(Fake::new(Service::Tidal, false));
        let error = sources
            .open(
                &track(vec![SourceRef::Spotify { id: "x".into() }]),
                Quality::Lossless,
                Duration::ZERO,
            )
            .await
            .err()
            .expect("unplayable");
        assert!(error.to_string().contains("bindings: spotify"), "{error}");
        let error = sources
            .open(&track(vec![]), Quality::Lossless, Duration::ZERO)
            .await
            .err()
            .expect("unplayable");
        assert!(error.to_string().contains("bindings: none"), "{error}");
    }
}
