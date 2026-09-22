//! The `Source` seam: turning a per-service [`SourceRef`] into decodable bytes.
//!
//! One implementation per service lives in its own crate (`canon-tidal` first, a
//! local-files source, and — proven feasible in yak canon-1175 — Spotify later). The
//! trait is intentionally ignorant of *how* bytes are fetched: the Tidal impl hides
//! DASH manifest resolution and transparent re-resolution of expired segment URLs
//! behind a plain seekable reader (yak canon-e99d).

use std::io::{Read, Seek};

use async_trait::async_trait;

use crate::{Quality, Result, Service, SourceRef, StreamInfo, TrackMeta};

/// A seekable byte input the decode stage (`canon-audio`, Symphonia) can consume.
///
/// Blanket-implemented for anything that is `Read + Seek + Send`, so both a plain
/// `File` and the Tidal segment reader satisfy it. `canon-audio` adapts a
/// `Box<dyn MediaInput>` into a Symphonia `MediaSource` at the boundary.
pub trait MediaInput: Read + Seek + Send {}
impl<T: Read + Seek + Send> MediaInput for T {}

/// A resolved, playable stream: the bytes plus their physical description.
pub struct ResolvedStream {
    pub input: Box<dyn MediaInput>,
    pub info: StreamInfo,
}

/// A music service (or the local filesystem) that resolves bindings to bytes and
/// answers metadata queries.
///
/// `async_trait` keeps this object-safe so the player can hold `Box<dyn Source>` and
/// dispatch by [`SourceRef::service`].
#[async_trait]
pub trait Source: Send + Sync {
    /// Which service this source serves.
    fn service(&self) -> Service;

    /// Resolve a binding to a decodable stream at up to the requested quality
    /// (clamped to what the account/backend can serve).
    async fn resolve(&self, source: &SourceRef, quality: Quality) -> Result<ResolvedStream>;

    /// Best-effort display metadata for a binding.
    async fn track_meta(&self, source: &SourceRef) -> Result<TrackMeta>;
}
