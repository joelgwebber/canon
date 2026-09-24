//! Tidal as a [`Source`] (and, in [`crate::catalog`], a `Catalog`): what the daemon's registry
//! plays and browses Tidal through.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canon_core::{Error, Quality, ResolvedStream, Result, Service, Source, SourceRef, SourceTrack};

use crate::TidalSession;
use crate::catalog::tidal_id;
use crate::connector::Observed;

/// A handle on a shared [`TidalSession`]. The session stays shared because the login verbs
/// drive the same one, and a stream's segment producer outlives the call that opened it.
pub struct TidalSource {
    session: Arc<TidalSession>,
    /// Where to report what playback shows about this login, when it came from the connector.
    observed: Option<Arc<Observed>>,
}

impl TidalSource {
    #[must_use]
    pub fn new(session: Arc<TidalSession>) -> Self {
        Self {
            session,
            observed: None,
        }
    }

    /// A source that reports what playback shows about its login: a refusal takes streaming away
    /// from it, a stream that opens confirms it.
    pub(crate) fn observed(session: Arc<TidalSession>, observed: Arc<Observed>) -> Self {
        Self {
            session,
            observed: Some(observed),
        }
    }

    pub(crate) fn session(&self) -> &TidalSession {
        &self.session
    }
}

#[async_trait]
impl Source for TidalSource {
    fn service(&self) -> Service {
        Service::Tidal
    }

    async fn open(
        &self,
        source: &SourceRef,
        quality: Quality,
        start: Duration,
    ) -> Result<ResolvedStream> {
        let opened = Arc::clone(&self.session)
            .open_stream_at(tidal_id(source)?, quality, start)
            .await;
        if let Some(observed) = &self.observed {
            match &opened {
                Ok(stream) => observed.streamed(&stream.info),
                // Tidal answers a login it won't let play with a 401/403 on playbackinfo.
                Err(Error::Auth(why)) => observed.refused(why),
                Err(_) => {}
            }
        }
        opened
    }

    async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
        self.session.describe(tidal_id(source)?).await
    }
}
