//! The engine's own count of frames the realtime callback has emitted.
//!
//! Private to the engine: the player actor never reads it. The engine turns it into
//! [`canon_core::EngineEvent::LocalPosition`] reports off the realtime thread (see
//! [`crate::engine`]), because the actor and the engine may not share a process (canon-5ce4).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A lock-free frame count shared between the realtime output callback (the sole writer of
/// emitted frames) and the engine's reporter. Frames are on the track's timeline at
/// `sample_rate`, so `frames / sample_rate` is position.
#[derive(Debug, Default)]
pub(crate) struct FrameClock {
    frames: AtomicU64,
    sample_rate: AtomicU64,
}

impl FrameClock {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Called from the realtime callback after emitting `frames` frames. Wait-free.
    pub(crate) fn advance(&self, frames: u64) {
        self.frames.fetch_add(frames, Ordering::Relaxed);
    }

    pub(crate) fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// Start counting a stream at `sample_rate` that begins `start` into its track.
    pub(crate) fn reset(&self, sample_rate: u32, start: Duration) {
        self.sample_rate
            .store(u64::from(sample_rate), Ordering::Relaxed);
        self.frames
            .store(frames_at(start, sample_rate), Ordering::Relaxed);
    }

    /// A gapless hand-off to the next track: the listener has just crossed from one track into
    /// the next at `boundary` frames, so position now counts from there. Subtracting (rather than
    /// resetting) keeps the frames the callback emitted since the crossing, whatever order the
    /// two threads run in. The rate is unchanged — a hand-off only joins tracks of one format.
    pub(crate) fn rebase(&self, boundary: u64) {
        let _ = self
            .frames
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |frames| {
                Some(frames.saturating_sub(boundary))
            });
    }

    /// Position derived from emitted frames. `Duration::ZERO` until a rate is set.
    pub(crate) fn position(&self) -> Duration {
        let sr = self.sample_rate.load(Ordering::Relaxed);
        if sr == 0 {
            return Duration::ZERO;
        }
        let frames = u128::from(self.frames.load(Ordering::Relaxed));
        let nanos = frames * 1_000_000_000 / u128::from(sr);
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }
}

/// How many frames at `sample_rate` make `position`. The engine counts the frames it feeds with
/// the same arithmetic, so its idea of where a track ends agrees with the clock's.
pub(crate) fn frames_at(position: Duration, sample_rate: u32) -> u64 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let frames = (position.as_secs_f64() * f64::from(sample_rate)).max(0.0) as u64;
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ported from canon-core's `frame_clock_math`: frames are position at the stream's rate,
    /// and a stream that starts mid-track counts from there.
    #[test]
    fn frame_clock_math() {
        let clock = FrameClock::new();
        assert_eq!(clock.position(), Duration::ZERO, "no rate, no position");

        clock.reset(48_000, Duration::ZERO);
        clock.advance(48_000);
        assert_eq!(clock.position(), Duration::from_secs(1));

        clock.reset(48_000, Duration::from_secs(2));
        assert_eq!(clock.position(), Duration::from_secs(2));
        clock.advance(24_000);
        assert_eq!(clock.position(), Duration::from_millis(2_500));
    }

    /// A hand-off keeps whatever the callback emitted past the boundary.
    #[test]
    fn rebasing_keeps_frames_past_the_boundary() {
        let clock = FrameClock::new();
        clock.reset(48_000, Duration::ZERO);
        clock.advance(10 * 48_000 + 4_800);
        clock.rebase(10 * 48_000);
        assert_eq!(clock.position(), Duration::from_millis(100));
    }
}
