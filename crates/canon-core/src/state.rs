//! The authoritative playback state, and the frame clock that keeps it honest.
//!
//! This module encodes the single most important lesson from tideway (bug tide-2f85):
//! the emitted "now playing" desynced from reality because position/liveness lived in
//! the realtime audio callback while transport state lived elsewhere, and the output
//! device could change under both without anything re-emitting.
//!
//! Canon's fix, in types:
//! * The realtime callback owns **no** clock-of-record. It only advances a
//!   [`FrameClock`] (frames actually emitted) and stamps the current device epoch.
//! * A control task derives position from `frames / sample_rate` on a fixed timer and
//!   publishes a seq-stamped [`PlayerSnapshot`]. A stalled callback is therefore
//!   *visible* (frames stop advancing) instead of an invisibly frozen emitter.
//!
//! The state *machine* (the actor that owns transitions and the emit loop) is built on
//! top of these types in yak canon-5afb.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{SinkId, TrackRef};

/// The discrete transport state. Only the state actor mutates it, and every mutation
/// bumps the snapshot `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
    Error,
}

/// A lock-free clock shared between the realtime output callback (the sole writer of
/// `frames`) and the control task (the sole reader that derives position + emits).
///
/// `epoch` bumps on every device/stream reopen and resets `frames`, so a rebased
/// counter after a device change can't be misread as forward progress — the reader
/// compares epochs before trusting a delta.
#[derive(Debug, Default)]
pub struct FrameClock {
    frames: AtomicU64,
    epoch: AtomicU64,
    /// Stored alongside so a reader converts frames→time without extra plumbing.
    sample_rate: AtomicU64,
}

impl FrameClock {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Called from the realtime callback after emitting `frames` frames. Wait-free.
    pub fn advance(&self, frames: u64) {
        self.frames.fetch_add(frames, Ordering::Relaxed);
    }

    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Relaxed)
    }

    /// Rebase for a new track: zero the emitted frames, record the source sample
    /// rate, and bump the epoch.
    pub fn reset(&self, sample_rate: u32) {
        self.frames.store(0, Ordering::Relaxed);
        self.sample_rate
            .store(u64::from(sample_rate), Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Seek discontinuity: set emitted frames to represent `position` (at the current
    /// source rate) and bump the epoch. Frames are source-timeline frames, so this is
    /// output-device independent.
    pub fn seek(&self, position: Duration) {
        let sr = self.sample_rate.load(Ordering::Relaxed);
        if sr == 0 {
            return;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let frames = (position.as_secs_f64() * sr as f64).max(0.0) as u64;
        self.frames.store(frames, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Output-device discontinuity that does NOT move the timeline: the same track
    /// keeps playing through a reopened stream, so frames continue accumulating and
    /// only the epoch advances. This is what lets a device-loss+reconnect re-emit with
    /// a *continuous* position instead of a frozen or reset one (the tide-2f85 fix).
    pub fn mark_device_change(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// Position derived from emitted frames. `Duration::ZERO` until a rate is set.
    #[must_use]
    pub fn position(&self) -> Duration {
        let sr = self.sample_rate.load(Ordering::Relaxed);
        if sr == 0 {
            return Duration::ZERO;
        }
        let frames = u128::from(self.frames.load(Ordering::Relaxed));
        let nanos = frames * 1_000_000_000 / u128::from(sr);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }

    /// Convenience for the wire snapshot (position in whole milliseconds).
    #[must_use]
    pub fn position_ms(&self) -> u64 {
        u64::try_from(self.position().as_millis()).unwrap_or(u64::MAX)
    }
}

/// The single authoritative view of playback, stamped with a monotonic `seq`.
///
/// Clients reconcile by `seq`; a late joiner gets a full snapshot then deltas.
/// Position is reported with a `rate` (0.0 paused, 1.0 playing) so clients interpolate
/// between snapshots without a high-frequency server poll (yak canon-9487).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub seq: u64,
    pub state: PlaybackState,
    pub track: Option<TrackRef>,
    pub position_ms: u64,
    pub duration_ms: Option<u64>,
    pub rate: f32,
    pub volume: f32,
    pub muted: bool,
    pub sink: Option<SinkId>,
    pub error: Option<String>,
}

impl PlayerSnapshot {
    /// The initial, nothing-loaded snapshot (`seq` 0).
    #[must_use]
    pub fn idle() -> Self {
        Self {
            seq: 0,
            state: PlaybackState::Idle,
            track: None,
            position_ms: 0,
            duration_ms: None,
            rate: 0.0,
            volume: 1.0,
            muted: false,
            sink: None,
            error: None,
        }
    }
}
