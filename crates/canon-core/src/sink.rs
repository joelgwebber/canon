//! The `Sink` seam: where mixed audio goes, and the routing that keeps local honest.
//!
//! Local speakers and LAN renderers are two ways to hear the same mix. The player treats
//! "which output" as ordinary state (yak canon-08a9) rather than a special case, and two
//! hard-won rules from tideway are encoded structurally here:
//!
//! * **Muting local while a network sink plays is done by *routing*.** [`OutputRoute`] hands
//!   the route to a network session; the *only* way local is restored is by dropping the
//!   returned [`RouteGuard`] (RAII). No separate "un-silence" step exists to be skipped, so a
//!   dead receiver or a missed teardown can't silence local output forever
//!   (tideway tide-4000.2/.3).
//! * **Teardown keys off *liveness*, not a discovery event.** A [`Sink`] publishes a
//!   [`SinkHealth`] stream; the player watches it and fails back to local the instant a sink
//!   reports [`SinkHealth::Failed`], rather than waiting for a "device removed" notice that a
//!   flaky renderer may never send.
//!
//! `local` is not itself a [`Sink`]: it is the *resting route*, active precisely when no
//! network sink holds the route. Selecting local means releasing the route; selecting a
//! renderer means taking it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

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

/// A selectable output, as reported to clients. This is the serialisable *description* of a sink
/// (what a picker lists), distinct from the live [`Sink`] object that drives one.
///
/// The local device is always listed, with the reserved id `local`; network entries come from
/// discovery. Keeping this in core lets the control plane publish a device list without the API
/// layer depending on any protocol implementation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SinkInfo {
    pub id: SinkId,
    pub name: String,
    pub kind: SinkKind,
}

impl SinkInfo {
    /// The reserved id of the local output.
    pub const LOCAL: &'static str = "local";

    /// The always-present local device entry.
    #[must_use]
    pub fn local() -> Self {
        Self {
            id: SinkId(Self::LOCAL.to_string()),
            name: "Local output".to_string(),
            kind: SinkKind::Local,
        }
    }

    /// Whether `id` names the local output.
    #[must_use]
    pub fn is_local(id: &SinkId) -> bool {
        id.0 == Self::LOCAL
    }
}

/// Liveness of a network sink. A renderer that stops responding transitions to `Failed`,
/// which the player consumes as a first-class state input (fail back to local) — never a
/// silent wedge. `Degraded` is a warning that stays playing (e.g. a slow reader, a recovered
/// reconnect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkHealth {
    Healthy,
    Degraded(String),
    Failed(String),
}

/// The control / transport plane of a **network** output (Chromecast, DLNA, …).
///
/// Implementations own their own connection and liveness detector. They publish health
/// through [`health`](Self::health) so the player can fail back to local without polling; a
/// clean `stop()` and the sink being dropped are both valid ends of a session, and either
/// one must release whatever [`RouteGuard`] the session held.
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

    /// A live view of the sink's health, updated by the sink's own liveness detector (Cast
    /// status timeout, DLNA poll, TCP close). The player watches this and, on
    /// [`SinkHealth::Failed`], drops the session and fails back to local — so teardown is
    /// driven by liveness, never by a discovery-remove event that may never arrive.
    fn health(&self) -> watch::Receiver<SinkHealth>;
}

/// The data plane: interleaved `f32` frames pushed from the realtime mix.
///
/// The local output writes to the device; network sinks feed their FLAC encoder / LAN stream
/// server. Implementations must never block the audio callback — they hand off to their own
/// buffer and return immediately.
pub trait PcmSink: Send {
    fn submit(&mut self, frames: &[f32], sample_rate: u32, channels: u16);
}

/// The realtime-visible half of the output route: a single lock-free word the local audio
/// callback reads every buffer to decide whether it owns the speakers this instant.
///
/// `0` is the resting value and means **local owns output**. Any non-zero value is the
/// *generation* of the network session that currently holds the route; while it is non-zero
/// the local callback emits silence instead of draining the mix (which the network sink is
/// consuming). The word is only ever set through [`OutputRoute::take`] and cleared by dropping
/// the returned [`RouteGuard`] — the callback never writes it.
#[derive(Debug, Default)]
pub struct LocalGate {
    /// 0 = local; non-zero = generation of the holding network session.
    holder: AtomicU64,
}

impl LocalGate {
    /// True while a network session holds the route (local should emit silence).
    #[must_use]
    pub fn local_muted(&self) -> bool {
        self.holder.load(Ordering::Acquire) != 0
    }
}

/// Owns the output route for the daemon's lifetime. Local speakers are the resting route;
/// handing the route to a network sink is done with [`take`](Self::take), which returns a
/// [`RouteGuard`] whose `Drop` is the *only* way local is restored. No teardown path, error,
/// or panic can therefore leave local silenced forever — the tideway tide-4000.x class of bug
/// is structurally impossible rather than merely avoided by careful code.
///
/// Clone it freely: every clone shares the one route.
#[derive(Debug, Clone)]
pub struct OutputRoute {
    gate: Arc<LocalGate>,
    /// Monotonic generation source, so each `take` gets a distinct token and a superseded
    /// guard's drop can be told apart from the current holder's.
    seq: Arc<AtomicU64>,
}

impl Default for OutputRoute {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputRoute {
    #[must_use]
    pub fn new() -> Self {
        Self {
            gate: Arc::new(LocalGate::default()),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The lock-free gate handed to the local audio callback. Cheap `Arc` clone.
    #[must_use]
    pub fn local_gate(&self) -> Arc<LocalGate> {
        Arc::clone(&self.gate)
    }

    /// True while a network session holds the route.
    #[must_use]
    pub fn local_muted(&self) -> bool {
        self.gate.local_muted()
    }

    /// Hand the route to a network session. Local goes silent immediately and stays silent
    /// until the returned guard is dropped.
    ///
    /// Taking the route again (switching from one renderer to another) *supersedes* the
    /// previous holder: the older guard's drop becomes a no-op, so a late teardown of the old
    /// session can't un-mute local out from under the new one. The player upholds the matching
    /// invariant — one active network session at a time, released in handoff order (take the
    /// new route, then drop the old guard).
    #[must_use]
    pub fn take(&self) -> RouteGuard {
        // fetch_add wraps only after 2^64 takes; +1 keeps 0 reserved for "local".
        let generation = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.gate.holder.store(generation, Ordering::Release);
        RouteGuard {
            gate: Arc::clone(&self.gate),
            generation,
        }
    }
}

/// Proof that a network session holds the output route (see [`OutputRoute::take`]). While it
/// lives, local output is muted; dropping it restores local. There is no manual un-mute to
/// forget — that is the whole point.
#[derive(Debug)]
pub struct RouteGuard {
    gate: Arc<LocalGate>,
    generation: u64,
}

impl Drop for RouteGuard {
    fn drop(&mut self) {
        // Clear the gate only if we're still the holder. If a newer `take` superseded us the
        // holder no longer equals our generation, the CAS fails, and we correctly leave the
        // route with the new session (whose own guard will clear it).
        let _ = self.gate.holder.compare_exchange(
            self.generation,
            0,
            Ordering::AcqRel,
            Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_owns_output_at_rest() {
        let route = OutputRoute::new();
        assert!(!route.local_muted());
        assert!(!route.local_gate().local_muted());
    }

    #[test]
    fn taking_the_route_mutes_local_and_dropping_restores_it() {
        let route = OutputRoute::new();
        let guard = route.take();
        assert!(route.local_muted(), "network session holds the route");
        drop(guard);
        assert!(!route.local_muted(), "dropping the guard restores local");
    }

    /// The tide-4000.x contract: even if the "stop the sink" code path is never reached, the
    /// guard going out of scope (here, an early return / panic unwinding a frame) restores
    /// local. Modelled by simply letting the guard drop at end of scope.
    #[test]
    fn forgotten_teardown_still_restores_local() {
        let route = OutputRoute::new();
        {
            let _guard = route.take();
            assert!(route.local_muted());
            // No explicit stop()/release here — the session code "forgot".
        }
        assert!(!route.local_muted(), "scope exit is the un-mute");
    }

    /// Switching renderers must not flicker local audible for even one buffer: take the new
    /// route first, then drop the old guard (handoff order), and local stays muted throughout.
    #[test]
    fn handoff_between_sinks_never_unmutes_local() {
        let route = OutputRoute::new();
        let first = route.take();
        assert!(route.local_muted());

        // New session supersedes the old before the old is torn down.
        let second = route.take();
        assert!(route.local_muted());

        // Old session's teardown lands late — must be a no-op, local stays muted.
        drop(first);
        assert!(route.local_muted(), "superseded guard must not un-mute");

        // Only releasing the current holder returns to local.
        drop(second);
        assert!(!route.local_muted());
    }

    #[test]
    fn a_superseded_guard_drop_is_a_noop() {
        let route = OutputRoute::new();
        let stale = route.take();
        let current = route.take();
        drop(stale); // stale generation no longer matches the holder
        assert!(route.local_muted(), "current holder still owns the route");
        drop(current);
        assert!(!route.local_muted());
    }

    // --- Sink trait: object-safety + liveness-as-event ------------------------------------

    /// A mock renderer that lets a test push health transitions, exactly as a real sink's
    /// liveness detector would.
    struct MockSink {
        id: SinkId,
        health: watch::Sender<SinkHealth>,
        health_rx: watch::Receiver<SinkHealth>,
    }

    impl MockSink {
        fn new(id: &str) -> Self {
            let (health, health_rx) = watch::channel(SinkHealth::Healthy);
            Self {
                id: SinkId(id.to_string()),
                health,
                health_rx,
            }
        }
        fn fail(&self, why: &str) {
            let _ = self.health.send(SinkHealth::Failed(why.to_string()));
        }
    }

    #[async_trait]
    impl Sink for MockSink {
        fn id(&self) -> SinkId {
            self.id.clone()
        }
        fn kind(&self) -> SinkKind {
            SinkKind::Chromecast
        }
        async fn start(&mut self, _meta: &TrackMeta) -> Result<()> {
            Ok(())
        }
        async fn pause(&mut self) -> Result<()> {
            Ok(())
        }
        async fn resume(&mut self) -> Result<()> {
            Ok(())
        }
        async fn stop(&mut self) -> Result<()> {
            Ok(())
        }
        async fn seek(&mut self, _position: Duration) -> Result<()> {
            Ok(())
        }
        async fn set_volume(&mut self, _volume: f32) -> Result<()> {
            Ok(())
        }
        fn health(&self) -> watch::Receiver<SinkHealth> {
            self.health_rx.clone()
        }
    }

    #[tokio::test]
    async fn sink_is_object_safe_and_publishes_liveness() {
        // Object safety: the player holds sinks as trait objects.
        let sink: Box<dyn Sink> = Box::new(MockSink::new("cast-1"));
        assert_eq!(sink.id(), SinkId("cast-1".to_string()));
        assert_eq!(sink.kind(), SinkKind::Chromecast);

        let health = sink.health();
        assert_eq!(*health.borrow(), SinkHealth::Healthy);

        // A concrete handle to drive the transition (the real detector lives inside the sink).
        let driver = MockSink::new("cast-1");
        let mut driver_health = driver.health();
        driver.fail("connection lost");
        driver_health.changed().await.expect("health transition");
        assert!(matches!(*driver_health.borrow(), SinkHealth::Failed(_)));
    }
}
