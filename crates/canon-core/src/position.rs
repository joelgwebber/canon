//! Where playback position comes from.
//!
//! Canon has two kinds of output and therefore two ways of knowing where we are, and
//! conflating them is a bug factory (tideway pulled stream position from whatever was
//! nearest to hand and paid for it forever):
//!
//! * **Local output** — we drive the device sample by sample, so the frames the realtime
//!   callback has emitted *are* the position. That is [`crate::FrameClock`], and its
//!   exactness is a feature: a stalled callback shows up as a frozen frame count rather
//!   than an emitter that has quietly stopped being true.
//! * **Network renderer** — the device plays on its own clock and buffers ahead of us, so
//!   the frames we have encoded and sent lead what the listener hears by the buffer depth
//!   (1–3s). The renderer itself is the only thing that knows where it is, and it tells us
//!   about twice a second. That is [`RendererClock`].
//!
//! [`PositionDrive`] is which of the two a given playback uses, settled at stream open.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Where a playback's position comes from.
///
/// Settled once per stream open, which is also once per output: switching outputs restarts
/// the stream at the current position (see `canon_audio::Output`), so "per stream" and "per
/// output" are the same thing and there is no way to end up mid-stream with the wrong drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionDrive {
    /// Frames emitted by the local realtime callback.
    Frames,
    /// A network renderer's own reports, extrapolated by wall time in between. On this path
    /// the frames we feed the encoder are *not* position — they run seconds ahead.
    Renderer,
}

/// The outcome of folding one renderer report into the clock. Returned rather than logged so
/// the clock needs no opinion about logging, and so tests can assert on the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconcile {
    /// Disagreement exceeded [`RendererClock::SNAP`]: position jumped to the reported value.
    Snapped { drift_ms: i64 },
    /// Routine disagreement, eased out over the next few reports.
    Slewed { by_ms: i64 },
}

/// Position for an output that plays on its own clock and reports back — a Chromecast
/// (`MEDIA_STATUS.currentTime`), a DLNA renderer (`AVTransport.GetPositionInfo`), and
/// eventually canon's own remote protocol.
///
/// Driving position from frames fed to the encoder is wrong by the renderer's buffer depth
/// and, worse, open-loop: nothing ever corrects it. This closes the loop. The renderer's
/// report is the authority; wall time carries the position between reports, which arrive far
/// too rarely (~2/sec) to display directly; [`reconcile`](Self::reconcile) folds each new
/// report in.
///
/// Corrections are deliberately *not* seeks. A seek is a user-visible discontinuity — it
/// bumps [`crate::FrameClock`]'s epoch and clients read that as "the user jumped" — whereas a
/// routine correction is just us sharpening an estimate. Applying reports verbatim twice a
/// second would make every client's progress bar twitch, so the common case slews instead.
#[derive(Debug, Clone)]
pub struct RendererClock {
    /// Where this stream begins on the source timeline. A renderer reports time relative to
    /// the media it was handed, and after a seek that media *starts* at the seek point, so
    /// every report has to be read against this to mean anything.
    origin: Duration,
    /// Position as of `running_since`, or the frozen position while stopped.
    anchor: Duration,
    /// When the anchor was taken. `None` while the renderer is not playing, which is what
    /// keeps buffering and paused time from counting as progress.
    running_since: Option<Instant>,
}

impl RendererClock {
    /// Beyond this much disagreement the renderer is somewhere genuinely different — it
    /// rebuffered, someone seeked it at the device, or our estimate was never right — and
    /// easing over there would take too long to watch. Snap instead.
    pub const SNAP: Duration = Duration::from_millis(1_500);

    /// Fraction of the drift taken per report when the renderer is *ahead* of us. Catching up
    /// runs the position forward, which is never jarring, so it can be brisk.
    const SLEW_AHEAD: i64 = 4;

    /// Fraction taken when the renderer is *behind* us. This walks the position backward, so
    /// it is gentler — and it absorbs renderers that quantise their reports (DLNA's `RelTime`
    /// is whole seconds, which reads as a permanent half-second lag) without visibly stepping
    /// backward twice a second.
    const SLEW_BEHIND: i64 = 8;

    /// A clock for a stream that begins at `origin` on the source timeline, not yet running.
    ///
    /// Stopped is the right initial state: the renderer has been handed a URL but is still
    /// buffering, and counting that as progress is what produces a jump backward the moment
    /// the first real report lands.
    #[must_use]
    pub fn new(origin: Duration) -> Self {
        Self {
            origin,
            anchor: origin,
            running_since: None,
        }
    }

    /// Position on the source timeline: the anchor plus however long we have been running.
    #[must_use]
    pub fn position(&self) -> Duration {
        match self.running_since {
            Some(since) => self.anchor + since.elapsed(),
            None => self.anchor,
        }
    }

    /// Convenience for the wire snapshot (whole milliseconds).
    #[must_use]
    pub fn position_ms(&self) -> u64 {
        u64::try_from(self.position().as_millis()).unwrap_or(u64::MAX)
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }

    /// Start counting wall time. Idempotent, so a caller can assert "running iff playing" on
    /// every transition without having to track edges.
    pub fn resume(&mut self) {
        if self.running_since.is_none() {
            self.running_since = Some(Instant::now());
        }
    }

    /// Stop counting, freezing the position where it is. Idempotent.
    pub fn pause(&mut self) {
        if self.running_since.is_some() {
            self.anchor = self.position();
            self.running_since = None;
        }
    }

    /// A real discontinuity: the stream now begins at `to`, which is also where we are.
    ///
    /// Seeking a network renderer means handing it a new stream starting at the seek point,
    /// so the origin moves with the position — otherwise the renderer's next report, which
    /// restarts near zero for the new media, would be read as a jump back to the top.
    pub fn seek(&mut self, to: Duration) {
        self.origin = to;
        self.jump_to(to);
    }

    /// Fold in a position the renderer reported for itself, measured from the start of the
    /// stream we gave it (Cast `currentTime`, DLNA `RelTime`).
    ///
    /// Shifting the *anchor* (rather than re-timing it) is what makes a correction invisible:
    /// the position moves by the correction and keeps running at the same rate, so clients
    /// interpolating from `rate` stay right.
    pub fn reconcile(&mut self, reported: Duration) -> Reconcile {
        let absolute = self.origin + reported;
        let drift_ms = millis(absolute) - millis(self.position());
        if drift_ms.abs() > millis(Self::SNAP) {
            self.jump_to(absolute);
            return Reconcile::Snapped { drift_ms };
        }
        let divisor = if drift_ms >= 0 {
            Self::SLEW_AHEAD
        } else {
            Self::SLEW_BEHIND
        };
        let step = drift_ms / divisor;
        self.shift(step);
        Reconcile::Slewed { by_ms: step }
    }

    /// Move to an absolute source-timeline position, preserving running/stopped.
    fn jump_to(&mut self, absolute: Duration) {
        self.anchor = absolute;
        if self.running_since.is_some() {
            self.running_since = Some(Instant::now());
        }
    }

    fn shift(&mut self, delta_ms: i64) {
        let magnitude = Duration::from_millis(delta_ms.unsigned_abs());
        self.anchor = if delta_ms >= 0 {
            self.anchor + magnitude
        } else {
            self.anchor.saturating_sub(magnitude)
        };
    }
}

/// Milliseconds as a signed count, so drift arithmetic can go either way without wrapping.
fn millis(d: Duration) -> i64 {
    i64::try_from(d.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn a_stopped_clock_does_not_advance() {
        let clock = RendererClock::new(at(5_000));
        assert_eq!(clock.position(), at(5_000));
        std::thread::sleep(at(20));
        assert_eq!(
            clock.position(),
            at(5_000),
            "buffering time must not count as progress"
        );
    }

    #[test]
    fn running_advances_and_pausing_freezes() {
        let mut clock = RendererClock::new(at(1_000));
        clock.resume();
        std::thread::sleep(at(30));
        clock.pause();
        let frozen = clock.position();
        assert!(frozen >= at(1_030), "expected progress, got {frozen:?}");
        std::thread::sleep(at(30));
        assert_eq!(clock.position(), frozen, "paused time must not accumulate");
    }

    #[test]
    fn resume_and_pause_are_idempotent() {
        let mut clock = RendererClock::new(at(0));
        clock.resume();
        let started = clock.position();
        std::thread::sleep(at(20));
        clock.resume(); // must not re-anchor and lose the elapsed time
        assert!(clock.position() > started);
        clock.pause();
        let frozen = clock.position();
        clock.pause();
        assert_eq!(clock.position(), frozen);
    }

    #[test]
    fn a_small_disagreement_slews_rather_than_jumping() {
        let mut clock = RendererClock::new(at(0));
        clock.jump_to(at(10_000));
        // Renderer is 400ms ahead of us: take a quarter of it, not all of it.
        let outcome = clock.reconcile(at(10_400));
        assert_eq!(outcome, Reconcile::Slewed { by_ms: 100 });
        assert_eq!(clock.position(), at(10_100));
    }

    #[test]
    fn being_ahead_of_the_renderer_eases_back_gently() {
        let mut clock = RendererClock::new(at(0));
        clock.jump_to(at(10_000));
        // Renderer reports 800ms behind us — which is also what a renderer that truncates
        // its reports to whole seconds looks like. Step back an eighth, not the whole way.
        let outcome = clock.reconcile(at(9_200));
        assert_eq!(outcome, Reconcile::Slewed { by_ms: -100 });
        assert_eq!(clock.position(), at(9_900));
    }

    #[test]
    fn repeated_slews_converge_on_the_renderer() {
        let mut clock = RendererClock::new(at(0));
        clock.jump_to(at(10_000));
        for _ in 0..24 {
            clock.reconcile(at(10_400));
        }
        let drift = millis(clock.position()) - millis(at(10_400));
        assert!(drift.abs() <= 5, "still {drift}ms out after 24 reports");
    }

    #[test]
    fn a_real_divergence_snaps() {
        let mut clock = RendererClock::new(at(0));
        clock.jump_to(at(10_000));
        // Someone seeked the device itself, or it rebuffered for seconds.
        let outcome = clock.reconcile(at(60_000));
        assert_eq!(outcome, Reconcile::Snapped { drift_ms: 50_000 });
        assert_eq!(clock.position(), at(60_000));
    }

    /// A renderer times the media it was handed, and after a seek that media starts at the
    /// seek point. Reading 0:04 of a stream that begins at 0:47 as "we are at 0:04" is what
    /// collapsed the position to the top of the track on real hardware.
    #[test]
    fn reports_are_read_against_the_streams_origin() {
        let mut clock = RendererClock::new(at(47_000));
        assert_eq!(clock.position(), at(47_000));

        // The device says it is 4s into the stream we gave it, i.e. 0:51 of the track.
        clock.reconcile(at(4_000));
        assert_eq!(clock.position(), at(51_000));
    }

    /// Seeking hands the renderer a fresh stream, so the origin has to travel with the
    /// position; leaving it behind makes the next report read as a jump to the top.
    #[test]
    fn seeking_moves_the_origin_with_the_position() {
        let mut clock = RendererClock::new(at(0));
        clock.resume();
        clock.seek(at(120_000));
        assert!(
            clock.position() >= at(120_000) && clock.position() < at(120_100),
            "seek landed at {:?}",
            clock.position()
        );
        assert!(clock.is_running(), "a seek must not stop playback");

        clock.reconcile(at(0));
        assert!(
            clock.position() >= at(119_000),
            "the new stream's zero was read as the top of the track: {:?}",
            clock.position()
        );
    }

    #[test]
    fn a_snap_keeps_the_clock_running() {
        let mut clock = RendererClock::new(at(0));
        clock.jump_to(at(10_000));
        clock.resume();
        clock.reconcile(at(60_000));
        assert!(clock.is_running(), "a correction must not stop playback");
    }

    #[test]
    fn slewing_never_walks_the_position_below_zero() {
        let mut clock = RendererClock::new(at(0));
        clock.jump_to(at(10));
        clock.reconcile(at(0));
        assert_eq!(clock.position(), at(9));
    }
}
