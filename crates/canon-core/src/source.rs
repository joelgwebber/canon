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
    Capability, Catalog, Connector, Error, PlayingFrom, Quality, Result, Service, SettingsStore,
    SourceRef, SourceTrack, StreamInfo, StreamingSettings, TrackMeta, TrackRef,
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
    /// The binding that was opened. [`Sources::open`] fills this in; a source may leave it.
    pub from: Option<SourceRef>,
    /// Set by a source whose bytes can be sought (from their start, `start_ms` 0) instead of
    /// fetched from a position: the engine decodes from this point of the track and reports
    /// where it really landed. Tidal's segments start near the position asked for instead, and
    /// leave this `None`.
    pub seek_to: Option<Duration>,
}

impl ResolvedStream {
    /// Where this stream's audio comes from, if the binding is known.
    #[must_use]
    pub fn playing_from(&self) -> Option<PlayingFrom> {
        self.from
            .clone()
            .map(|source| PlayingFrom::new(source, &self.info))
    }
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

/// Where tracks come from: the services' connectors, plus any sources registered directly (local
/// files, tests), and the policy for choosing among a track's bindings.
///
/// Routing is by capability (docs/connections.md): a binding opens only through a connection that
/// streams, browsing only through one with catalog access. When a service is connected but not
/// for what is asked, the answer is [`Error::NotEntitled`] with what to do about it, not whatever
/// the service said when tried anyway.
#[derive(Default, Clone)]
pub struct Sources {
    by_service: HashMap<Service, Arc<dyn Source>>,
    catalogs: HashMap<Service, Arc<dyn Catalog>>,
    connectors: Vec<Arc<dyn Connector>>,
    /// Where the user's streaming preference is read from, at each use; the default order when
    /// there is none.
    settings: Option<Arc<dyn SettingsStore>>,
}

impl Sources {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `source` for its service, always available, replacing any earlier one.
    #[must_use]
    pub fn with(mut self, source: Arc<dyn Source>) -> Self {
        self.by_service.insert(source.service(), source);
        self
    }

    /// Register `catalog` for its service, always available, replacing any earlier one.
    #[must_use]
    pub fn with_catalog(mut self, catalog: Arc<dyn Catalog>) -> Self {
        self.catalogs.insert(catalog.service(), catalog);
        self
    }

    /// Register a service's connector: its sources and catalog follow its connections.
    #[must_use]
    pub fn with_connector(mut self, connector: Arc<dyn Connector>) -> Self {
        self.connectors.push(connector);
        self
    }

    /// Read the streaming preference (`streaming.order`) from `settings`, at each use, so a
    /// change applies without a restart.
    #[must_use]
    pub fn with_preference(mut self, settings: Arc<dyn SettingsStore>) -> Self {
        self.settings = Some(settings);
        self
    }

    /// The user's streaming preference.
    #[must_use]
    pub fn preference(&self) -> StreamingSettings {
        self.settings
            .as_ref()
            .map(|settings| settings.get().streaming)
            .unwrap_or_default()
    }

    /// Every registered connector.
    #[must_use]
    pub fn connectors(&self) -> &[Arc<dyn Connector>] {
        &self.connectors
    }

    /// The connectors for `service`. A service can have several, one per way in with its own
    /// client: Spotify's Web API for its library, librespot for its audio.
    pub fn connectors_for(&self, service: Service) -> impl Iterator<Item = &Arc<dyn Connector>> {
        self.connectors
            .iter()
            .filter(move |c| c.service() == service)
    }

    /// Why nothing for `service` grants `capability`, from the connector that is meant to (one
    /// of whose methods grants it), else the first; `None` if the service has no connector.
    fn refusal(&self, service: Service, capability: Capability) -> Option<Error> {
        let meant = self.connectors_for(service).find(|connector| {
            connector
                .methods()
                .iter()
                .any(|method| method.grants.has(capability))
        });
        meant
            .or_else(|| self.connectors_for(service).next())
            .map(|connector| not_entitled(connector, capability))
    }

    /// The service to browse when a client doesn't say: the first one that can be.
    #[must_use]
    pub fn default_catalog(&self) -> Option<Service> {
        self.catalogs.keys().next().copied().or_else(|| {
            self.connectors
                .iter()
                .find(|c| c.grants(Capability::Catalog))
                .map(|c| c.service())
        })
    }

    /// The catalog for `service`.
    ///
    /// # Errors
    /// [`Error::NotEntitled`] if the service is connected without catalog access; `Unsupported`
    /// if it can't be browsed at all.
    pub fn catalog(&self, service: Service) -> Result<Arc<dyn Catalog>> {
        if let Some(catalog) = self.catalogs.get(&service) {
            return Ok(Arc::clone(catalog));
        }
        if let Some(catalog) = self.connectors_for(service).find_map(|c| c.catalog()) {
            return Ok(catalog);
        }
        Err(self
            .refusal(service, Capability::Catalog)
            .unwrap_or_else(|| Error::Unsupported(format!("{service} can't be browsed"))))
    }

    /// The source to use for a `service` binding, for `need`. `Ok(None)` when canon has nothing
    /// for that service at all, so its bindings are passed over rather than reported.
    fn source(&self, service: Service, need: Capability) -> Result<Option<Arc<dyn Source>>> {
        if let Some(source) = self.by_service.get(&service) {
            return Ok(Some(Arc::clone(source)));
        }
        if let Some(source) = self.connectors_for(service).find_map(|c| c.source(need)) {
            return Ok(Some(source));
        }
        match self.refusal(service, need) {
            Some(refused) => Err(refused),
            None => Ok(None),
        }
    }

    /// Open the first of `track`'s bindings that will open, in policy order.
    ///
    /// # Errors
    /// Every candidate's failure (a service connected but not for streaming says so), or that
    /// the track has no binding at all.
    pub async fn open(
        &self,
        track: &TrackRef,
        quality: Quality,
        start: Duration,
    ) -> Result<ResolvedStream> {
        let mut failures = Vec::new();
        for binding in self.in_preference_order(&track.sources) {
            let service = binding.service();
            let opened = match self.source(service, Capability::Stream) {
                Ok(Some(source)) => source.open(binding, quality, start).await,
                Ok(None) => continue,
                Err(e) => Err(e),
            };
            match opened {
                Ok(mut stream) => {
                    stream.from = Some(binding.clone());
                    return Ok(stream);
                }
                Err(e) => failures.push((service, e)),
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
        for binding in self.in_preference_order(&track.sources) {
            let described = match self.source(binding.service(), Capability::Catalog) {
                Ok(Some(source)) => source.describe(binding).await,
                Ok(None) => continue,
                Err(e) => Err(e),
            };
            match described {
                Ok(described) => return Ok(described.meta()),
                Err(e) => failures.push((binding.service(), e)),
            }
        }
        Err(Self::unplayable(track, failures))
    }

    /// What the service behind `binding` says about it. Needs only catalog access.
    ///
    /// # Errors
    /// Nothing can describe the binding's service, or it failed to.
    pub async fn describe(&self, binding: &SourceRef) -> Result<SourceTrack> {
        let service = binding.service();
        match self.source(service, Capability::Catalog)? {
            Some(source) => source.describe(binding).await,
            None => Err(Error::Source(format!("no {service} source is available"))),
        }
    }

    /// Whether canon can stream `service` now: a source registered for it directly, or a
    /// connection that grants [`Capability::Stream`]. Local and cheap; asks the service nothing.
    #[must_use]
    pub fn can_stream(&self, service: Service) -> bool {
        self.by_service.contains_key(&service)
            || self
                .connectors_for(service)
                .any(|connector| connector.grants(Capability::Stream))
    }

    /// Every service canon can stream now, in the user's preference order (services it doesn't
    /// rank keep their registration order, then sources registered directly by name).
    #[must_use]
    pub fn streaming_services(&self) -> Vec<Service> {
        let mut services: Vec<Service> = self
            .connectors
            .iter()
            .filter(|connector| connector.grants(Capability::Stream))
            .map(|connector| connector.service())
            .collect();
        let mut direct: Vec<Service> = self
            .by_service
            .keys()
            .copied()
            .filter(|service| !services.contains(service))
            .collect();
        direct.sort_by_key(|service| service.as_str());
        services.extend(direct);
        let preference = self.preference();
        services.sort_by_key(|service| preference.rank(*service));
        // Several connectors can stream one service; list it once. (After the sort, which is
        // stable and keys on the service, so duplicates sit together.)
        services.dedup();
        services
    }

    /// The service a track with these bindings would play from, as far as canon can tell without
    /// trying: the first binding in policy order on a service it can stream. `None` when nothing
    /// bound is streamable (opening would fail before asking any service).
    #[must_use]
    pub fn plays_from(&self, bindings: &[SourceRef]) -> Option<Service> {
        self.in_preference_order(bindings)
            .into_iter()
            .map(SourceRef::service)
            .find(|service| self.can_stream(*service))
    }

    /// `bindings` in the order to try them: local files first (no network, no account), then by
    /// the user's streaming preference, and otherwise in the track's own order.
    fn in_preference_order<'a>(&self, bindings: &'a [SourceRef]) -> Vec<&'a SourceRef> {
        let preference = self.preference();
        let mut bindings: Vec<&SourceRef> = bindings.iter().collect();
        // Stable, so the track's own order breaks ties.
        bindings.sort_by_key(|binding| preference.rank(binding.service()));
        bindings
    }

    /// Why nothing played. One failure is returned as it was, so its kind (an expired login, a
    /// connection that can't stream) survives; several are summarised together.
    fn unplayable(track: &TrackRef, mut failures: Vec<(Service, Error)>) -> Error {
        if failures.len() == 1 {
            return failures.remove(0).1;
        }
        if failures.is_empty() {
            let services: Vec<&str> = track.sources.iter().map(|s| s.service().as_str()).collect();
            return Error::Source(format!(
                "no source can play this track (bindings: {})",
                if services.is_empty() {
                    "none".to_string()
                } else {
                    services.join(", ")
                }
            ));
        }
        let each: Vec<String> = failures
            .iter()
            .map(|(service, e)| format!("{service}: {e}"))
            .collect();
        Error::Source(each.join("; "))
    }
}

fn not_entitled(connector: &Arc<dyn Connector>, capability: Capability) -> Error {
    Error::NotEntitled {
        service: connector.service(),
        capability,
        hint: connector.hint(capability),
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
                from: None,
                seek_to: None,
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

    /// A connector whose one login grants `grants`, serving everything from one fake source.
    struct FakeConnector {
        grants: crate::Capabilities,
        source: Arc<Fake>,
    }

    #[async_trait]
    impl Connector for FakeConnector {
        fn service(&self) -> Service {
            Service::Tidal
        }
        fn methods(&self) -> Vec<crate::Method> {
            Vec::new()
        }
        async fn connections(&self) -> Vec<crate::ConnectionInfo> {
            Vec::new()
        }
        async fn begin(&self, _method: &str) -> Result<crate::LoginFlow> {
            Err(Error::Unsupported("fake".into()))
        }
        async fn complete(
            &self,
            _method: &str,
            _input: Option<String>,
        ) -> Result<crate::LoginStatus> {
            Err(Error::Unsupported("fake".into()))
        }
        async fn disconnect(&self, _method: &str) -> Result<()> {
            Ok(())
        }
        fn grants(&self, capability: Capability) -> bool {
            self.grants.has(capability)
        }
        fn source(&self, need: Capability) -> Option<Arc<dyn Source>> {
            self.grants
                .has(need)
                .then(|| Arc::clone(&self.source) as Arc<dyn Source>)
        }
        fn catalog(&self) -> Option<Arc<dyn Catalog>> {
            None
        }
        fn hint(&self, capability: Capability) -> String {
            format!("sign in to {capability}")
        }
    }

    /// A library-only login describes a track but won't open it, and says why instead of letting
    /// the service fail however it does.
    #[tokio::test]
    async fn a_connection_that_cannot_stream_says_so() {
        let fake = Fake::new(Service::Tidal, false);
        let sources = Sources::new().with_connector(Arc::new(FakeConnector {
            grants: crate::Capabilities {
                catalog: true,
                ..crate::Capabilities::default()
            },
            source: Arc::clone(&fake),
        }));
        let track = track(vec![tidal("1")]);

        assert_eq!(
            sources.track_meta(&track).await.unwrap().title,
            "from tidal"
        );
        let error = sources
            .open(&track, Quality::Lossless, Duration::ZERO)
            .await
            .err()
            .expect("not entitled");
        assert!(
            matches!(
                error,
                Error::NotEntitled {
                    capability: Capability::Stream,
                    ..
                }
            ),
            "{error}"
        );
        assert_eq!(error.to_string(), "tidal can't stream: sign in to stream");
        assert!(fake.opened.lock().unwrap().is_empty(), "never tried");
        assert!(matches!(
            sources.catalog(Service::Tidal),
            Err(Error::NotEntitled { .. })
        ));
    }

    /// Only a direct source or a connection that grants streaming counts as streamable, and a
    /// local file is where a track would play from before any service.
    #[test]
    fn what_can_stream_is_known_without_asking() {
        let browse_only = Sources::new().with_connector(Arc::new(FakeConnector {
            grants: crate::Capabilities {
                catalog: true,
                ..crate::Capabilities::default()
            },
            source: Fake::new(Service::Tidal, false),
        }));
        assert!(!browse_only.can_stream(Service::Tidal));
        assert!(browse_only.streaming_services().is_empty());
        assert_eq!(browse_only.plays_from(&[tidal("1")]), None);

        let sources = Sources::new()
            .with_connector(Arc::new(FakeConnector {
                grants: crate::Capabilities {
                    stream: Some(Quality::Lossless),
                    ..crate::Capabilities::default()
                },
                source: Fake::new(Service::Tidal, false),
            }))
            .with(Fake::new(Service::Local, false));
        assert!(sources.can_stream(Service::Tidal));
        assert!(!sources.can_stream(Service::Spotify));
        assert_eq!(
            sources.streaming_services(),
            vec![Service::Local, Service::Tidal],
            "by preference: local files always first"
        );
        let spotify = SourceRef::Spotify { id: "x".into() };
        assert_eq!(sources.plays_from(std::slice::from_ref(&spotify)), None);
        assert_eq!(
            sources.plays_from(&[spotify, tidal("1"), local("/a.flac")]),
            Some(Service::Local)
        );
    }

    #[tokio::test]
    async fn a_streaming_connection_opens() {
        let fake = Fake::new(Service::Tidal, false);
        let sources = Sources::new().with_connector(Arc::new(FakeConnector {
            grants: crate::Capabilities {
                catalog: true,
                stream: Some(Quality::HiRes),
                ..crate::Capabilities::default()
            },
            source: Arc::clone(&fake),
        }));
        sources
            .open(&track(vec![tidal("1")]), Quality::Lossless, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(fake.opened.lock().unwrap().len(), 1);
    }

    /// Settings held in memory, for the preference tests.
    struct Held(Mutex<crate::Settings>);

    #[async_trait]
    impl SettingsStore for Held {
        fn get(&self) -> crate::Settings {
            self.0.lock().unwrap().clone()
        }
        async fn set(&self, settings: crate::Settings) -> Result<()> {
            *self.0.lock().unwrap() = settings;
            Ok(())
        }
    }

    /// Two services could play the track: the one the user prefers does, whatever order the
    /// track lists its bindings in, and a changed preference applies at once.
    #[tokio::test]
    async fn the_preferred_service_plays() {
        let tidal_source = Fake::new(Service::Tidal, false);
        let spotify_source = Fake::new(Service::Spotify, false);
        let settings = Arc::new(Held(Mutex::new(crate::Settings::default())));
        let sources = Sources::new()
            .with(spotify_source.clone())
            .with(tidal_source.clone())
            .with_preference(settings.clone());
        let spotify = SourceRef::Spotify { id: "s".into() };
        let track = track(vec![spotify.clone(), tidal("1")]);

        sources
            .open(&track, Quality::Lossless, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(
            tidal_source.opened.lock().unwrap().len(),
            1,
            "Tidal by default"
        );
        assert_eq!(sources.plays_from(&track.sources), Some(Service::Tidal));
        assert_eq!(
            sources.streaming_services(),
            vec![Service::Tidal, Service::Spotify]
        );

        let mut changed = crate::Settings::default();
        changed.streaming.order = vec![Service::Spotify, Service::Tidal];
        settings.set(changed).await.unwrap();
        sources
            .open(&track, Quality::Lossless, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(spotify_source.opened.lock().unwrap().len(), 1);
        assert_eq!(sources.plays_from(&track.sources), Some(Service::Spotify));
    }
}
