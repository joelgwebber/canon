//! Tidal as a [`Source`]: what the daemon's source registry plays Tidal bindings through.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canon_core::{Error, Quality, ResolvedStream, Result, Service, Source, SourceRef, SourceTrack};

use crate::TidalSession;

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
}

/// The Tidal id in a binding, or why this source can't take it.
fn tidal_id(source: &SourceRef) -> Result<&str> {
    match source {
        SourceRef::Tidal { id } => Ok(id),
        other => Err(Error::Unsupported(format!(
            "canon-tidal cannot play a {} source",
            other.service()
        ))),
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
