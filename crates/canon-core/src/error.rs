//! The one error type crossing canon's internal seams.
//!
//! Kept deliberately small and domain-shaped. Peripheral crates map their own
//! failures (HTTP, decode, SOAP, sqlite) into these variants at the boundary so the
//! player and API layers reason about a single `Result`.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    /// A music `Source` (Tidal, local files, …) failed to resolve or read.
    #[error("source: {0}")]
    Source(String),

    /// An output `Sink` (local device, Cast, DLNA) failed a control or data op.
    #[error("sink: {0}")]
    Sink(String),

    /// Authentication / token lifecycle failure (permanent — caller should re-auth).
    #[error("auth: {0}")]
    Auth(String),

    /// A transient, retryable failure (rate limit, 5xx, a receiver that fell behind).
    #[error("transient: {0}")]
    Transient(String),

    /// The requested operation isn't supported by this backend (e.g. a codec/tier).
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// The library's store failed, or a write would contradict what it holds (a binding
    /// already claimed by another entity).
    #[error("library: {0}")]
    Library(String),

    /// No connection to the service allows this: none at all, or a Tidal login that can browse
    /// but not stream, say. `hint` says what would.
    #[error("{service} can't {capability}: {hint}")]
    NotEntitled {
        service: crate::Service,
        capability: crate::Capability,
        hint: String,
    },

    /// The referenced entity/track/device could not be found.
    #[error("not found: {0}")]
    NotFound(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}
