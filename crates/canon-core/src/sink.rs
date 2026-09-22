//! The `Sink` seam: where mixed audio goes.
//!
//! Local speakers and LAN renderers implement the same control-plane trait, so the
//! player treats "which output" as ordinary state (yak canon-08a9) rather than a
//! special case. Two hard-won rules from tideway are encoded structurally here:
//!
//! * Muting local while casting is done by *routing*, and un-routing is tied to
//!   dropping the session (RAII), so a skipped teardown can't silence local output
//!   forever (tideway tide-4000.2/.3).
//! * A sink exposes a [`SinkHealth`] liveness signal; teardown/failover keys off that,
//!   not off a discovery "device removed" event that may never arrive.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Result, TrackMeta};

/// Stable identifier for a discovered output (device UUID, "local", …).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SinkId(pub String);

/// The kind of output, for UI grouping and protocol-specific behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkKind {
    Local,
    Chromecast,
    Dlna,
    // OpenHome / Tidal Connect land later.
}

/// Liveness of a sink. A network renderer that stops responding transitions to
/// `Failed`, which the player consumes as a first-class state input — never a silent
/// wedge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkHealth {
    Healthy,
    Degraded(String),
    Failed(String),
}

/// The control / transport plane of an output.
#[async_trait]
pub trait Sink: Send + Sync {
    fn id(&self) -> SinkId;
    fn kind(&self) -> SinkKind;

    /// Begin a playback session (spin up the stream server, issue the load, …).
    async fn start(&mut self, meta: &TrackMeta) -> Result<()>;
    async fn pause(&mut self) -> Result<()>;
    async fn resume(&mut self) -> Result<()>;
    async fn stop(&mut self) -> Result<()>;
    async fn seek(&mut self, position: Duration) -> Result<()>;
    async fn set_volume(&mut self, volume: f32) -> Result<()>;

    /// Current liveness. Polled by the player to drive failover to local.
    fn health(&self) -> SinkHealth;
}

/// The data plane: interleaved `f32` frames pushed from the realtime mix.
///
/// The local sink writes to the device; network sinks feed their FLAC encoder / LAN
/// stream server. Implementations must never block the audio callback — they hand off
/// to their own buffer and return immediately.
pub trait PcmSink: Send {
    fn submit(&mut self, frames: &[f32], sample_rate: u32, channels: u16);
}
