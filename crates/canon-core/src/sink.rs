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
//!   (tideway tide-4000.2/.3). Reserved for simultaneous output (canon-0205): with one active
//!   output, local is silent simply because it is not selected.
//! * **The renderer's own reports are the authority.** A [`Sink`] is a command surface, and
//!   everything the device says about itself comes back as a [`RendererEvent`] stream, which the
//!   daemon feeds into the player as engine events. Nothing about playback state is inferred
//!   from having *sent* a command, and teardown keys off that stream (a failure, a takeover, or
//!   the stream closing) rather than a discovery "device removed" notice that a flaky renderer
//!   may never send.
//!
//! `local` is not itself a [`Sink`]: it is the *resting route*, active precisely when no
//! network sink holds the route. Selecting local means releasing the route; selecting a
//! renderer means taking it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

/// What a network renderer says it is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererState {
    Playing,
    Paused,
    /// Filling its buffer — playback is genuinely not progressing, so this is reported as
    /// `Loading` rather than hidden, and the renderer clock stops for the duration.
    Buffering,
}

/// What a network renderer reported about itself, in protocol-neutral terms. Each protocol
/// (Cast `MEDIA_STATUS`, DLNA `GetTransportInfo`/`LastChange`) only has to *classify* its own
/// reports into these; everything downstream is shared.
///
/// These are inputs to the player state machine, never confirmations of our own commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RendererEvent {
    /// A condition the renderer is in. Reported on every poll for as long as it holds: only the
    /// player knows whether it is news.
    State(RendererState),
    /// Where the renderer says it is, relative to the start of the stream it was handed.
    Position(Duration),
    /// Our media finished normally.
    Ended,
    /// Something else took the renderer — another sender, another protocol, or the device
    /// evicting us. We must stop asserting control and fail back.
    Superseded(String),
    /// The renderer reported an error, a command was rejected, or the connection died.
    Failed(String),
}

impl RendererEvent {
    /// Whether this is a one-shot edge rather than an ongoing condition. Edges drive actions that
    /// must happen exactly once (auto-advance, fail-back), so a continuous poll must not keep
    /// re-firing them; conditions and positions must keep flowing.
    #[must_use]
    pub fn is_edge(&self) -> bool {
        matches!(
            self,
            RendererEvent::Ended | RendererEvent::Superseded(_) | RendererEvent::Failed(_)
        )
    }
}

/// Which [`Sink::load`] a report describes. Assigned by the sink, increasing per session.
///
/// A renderer keeps reporting on the media it has while the next load is still in flight, so a
/// report's meaning depends on *which* stream it is about: "ended" for a track the user already
/// skipped past must not advance the queue again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LoadId(pub u64);

/// One renderer report, attributed to the load whose media it describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererReport {
    pub load: LoadId,
    pub event: RendererEvent,
}

/// The control plane of a **network** output (Chromecast, DLNA, …): what we can ask it to do.
///
/// Commands are fire-and-forget and synchronous: an implementation queues them to the task that
/// owns its connection and returns at once, so the caller never blocks on the network (and can
/// issue them while holding its own state lock). `Err` means the command could not even be queued
/// — the connection is gone. Anything that goes wrong *after* that arrives on the sink's
/// [`RendererEvent`] stream, which is also where every consequence of a command shows up: a
/// successful `pause` is observed as the device reporting `Paused`, not assumed.
///
/// Seeking is deliberately absent. A renderer is fed a live stream, so seeking means handing it a
/// fresh stream that starts at the seek point — a [`load`](Self::load), not a transport command.
///
/// Dropping a sink ends its session: the implementation stops the renderer and disconnects.
pub trait Sink: Send + Sync {
    fn id(&self) -> SinkId;
    fn kind(&self) -> SinkKind;

    /// Point the renderer at a stream URL (our LAN stream server) and start it. Issued again for
    /// every new stream: a track change, a seek, a switch onto this sink. Reports about this
    /// stream will carry the returned [`LoadId`].
    fn load(&self, url: &str, meta: &TrackMeta) -> Result<LoadId>;
    fn play(&self) -> Result<()>;
    fn pause(&self) -> Result<()>;
    fn stop(&self) -> Result<()>;
    /// Set the renderer's own volume (0.0–1.0). The renderer owns volume; canon never scales the
    /// PCM it streams to one.
    fn set_volume(&self, volume: f32) -> Result<()>;
    fn set_muted(&self, muted: bool) -> Result<()>;
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

    // --- Sink trait: object safety + edge classification -----------------------------------

    /// A renderer that accepts every command, standing in for a real protocol impl.
    struct MockSink;

    impl Sink for MockSink {
        fn id(&self) -> SinkId {
            SinkId("cast-1".to_string())
        }
        fn kind(&self) -> SinkKind {
            SinkKind::Chromecast
        }
        fn load(&self, _url: &str, _meta: &TrackMeta) -> Result<LoadId> {
            Ok(LoadId(1))
        }
        fn play(&self) -> Result<()> {
            Ok(())
        }
        fn pause(&self) -> Result<()> {
            Ok(())
        }
        fn stop(&self) -> Result<()> {
            Ok(())
        }
        fn set_volume(&self, _volume: f32) -> Result<()> {
            Ok(())
        }
        fn set_muted(&self, _muted: bool) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn sink_is_object_safe() {
        // The daemon holds the active renderer as a trait object, whatever its protocol.
        let sink: Box<dyn Sink> = Box::new(MockSink);
        assert_eq!(sink.id(), SinkId("cast-1".to_string()));
        assert_eq!(sink.kind(), SinkKind::Chromecast);
        assert!(sink.pause().is_ok());
    }

    #[test]
    fn only_one_shot_reports_are_edges() {
        assert!(RendererEvent::Ended.is_edge());
        assert!(RendererEvent::Superseded("x".into()).is_edge());
        assert!(RendererEvent::Failed("x".into()).is_edge());
        assert!(!RendererEvent::State(RendererState::Playing).is_edge());
        assert!(!RendererEvent::Position(Duration::from_secs(1)).is_edge());
    }
}
