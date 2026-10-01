//! The `Exporter` seam: the one thing canon writes to a service (yak canon-8ed0).
//!
//! [`Catalog`](crate::Catalog) reads a service; this writes to it, and only in one direction:
//! **create a new playlist**. There is no update and no delete, by design (yak canon-65f7) —
//! canon never changes or removes something the user already has upstream, so a bad export costs
//! them one hand-deletion and never a lost playlist. Re-exporting makes another new playlist;
//! the description carries a stamp so the older ones are easy to find.
//!
//! Gated like [`Source`](crate::Source) and [`Catalog`](crate::Catalog): a connector hands one
//! out only from a connection granting [`Capability::LibraryWrite`](crate::Capability), and
//! [`Sources::exporter`](crate::Sources::exporter) says why when none does.

use async_trait::async_trait;

use crate::{Result, Service, SourceRef};

/// A service canon can create a playlist on.
#[async_trait]
pub trait Exporter: Send + Sync {
    /// Which service this writes to.
    fn service(&self) -> Service;

    /// Create a **new** playlist named `name`, described `description`, holding `tracks` in
    /// order, and answer with the service's binding for it. Never touches an existing playlist,
    /// whatever it is called.
    ///
    /// `tracks` are bindings on this service (what [`Catalog`](crate::Catalog) and matching
    /// produce); a binding on another service is an [`Error::Unsupported`](crate::Error).
    ///
    /// # Errors
    /// The service refused the creation or any of the additions. A partly filled playlist may be
    /// left behind: canon doesn't delete upstream, so the error says what to clean up by hand.
    async fn create_playlist(
        &self,
        name: &str,
        description: &str,
        tracks: &[SourceRef],
    ) -> Result<SourceRef>;
}
