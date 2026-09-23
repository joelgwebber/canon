//! Tidal as a [`Source`] (and, in [`crate::catalog`], a `Catalog`): what the daemon's registry
//! plays and browses Tidal through.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canon_core::{Quality, ResolvedStream, Result, Service, Source, SourceRef, SourceTrack};

use crate::TidalSession;
use crate::catalog::tidal_id;

/// A handle on a shared [`TidalSession`]. The session stays shared because the login verbs
/// drive the same one, and a stream's segment producer outlives the call that opened it.
pub struct TidalSource {
    session: Arc<TidalSession>,
}

impl TidalSource {
    #[must_use]
    pub fn new(session: Arc<TidalSession>) -> Self {
        Self { session }
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
        Arc::clone(&self.session)
            .open_stream_at(tidal_id(source)?, quality, start)
            .await
    }

    async fn describe(&self, source: &SourceRef) -> Result<SourceTrack> {
        self.session.describe(tidal_id(source)?).await
    }
}
