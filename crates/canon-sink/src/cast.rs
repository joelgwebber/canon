//! Chromecast sink: connect, LOAD, and status feedback (yak canon-dde4).
//!
//! Drives a Cast receiver to pull our LAN FLAC stream ([`crate::stream_server`]) and reports what
//! the device says back into the player's state machine.
//!
//! ## The tideway lesson this encodes
//!
//! tideway asserted control and *assumed* the device obeyed: it tracked cast state in its own
//! flags, so when something else took the receiver — a phone casting Spotify to the same speaker,
//! someone hitting pause on the device, the app being evicted — the UI kept showing tideway's
//! stale idea of reality while the speaker did something else entirely. Here the device's own
//! `MEDIA_STATUS` is the authority: every status frame is classified into a
//! [`RendererEvent`] and fed back through the player state actor as an ordinary input, exactly like
//! a local device change.
//! Nothing about playback state is inferred from the fact that we *sent* a command.
//!
//! ## Why one thread owns the connection, and how commands get in
//!
//! `rust_cast` is a blocking client whose `MessageManager` holds a single mutex over the TLS
//! stream, and its `read` keeps that mutex for the whole blocking read. A second thread issuing a
//! command while the first waits on `receive()` therefore deadlocks — and each command is itself a
//! send-then-read round trip. So exactly one thread owns the [`CastDevice`] and performs *all*
//! I/O; the async world talks to it only through channels.
//!
//! That thread cannot both block in `receive()` and notice a queued command, and `rust_cast`
//! exposes neither its socket nor a read timeout, so a bounded read is not available through its
//! API. Instead the thread alternates: it drains queued commands, then makes exactly one blocking
//! *request* — a `get_status` poll every [`POLL_INTERVAL`] — which returns the device's current
//! status and, as a side effect, buffers any unsolicited frames (pings, spontaneous status) that
//! arrived meanwhile for the following `receive()` calls to drain without blocking. So command
//! latency is bounded by the poll interval rather than by device silence, and no command can wedge
//! behind an idle socket.
//!
//! The poll is therefore load-bearing, not a workaround for raciness: it is what makes a
//! single-threaded blocking client responsive. It also yields the device-reported position that a
//! later yak reconciles the clock against.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::time::Duration;

use canon_core::{
    Error, LoadId, RendererEvent, RendererReport, RendererState, Result, Sink, SinkId, SinkKind,
    TrackMeta,
};
use rust_cast::CastDevice;
use rust_cast::channels::media::Status as MediaStatus;
use rust_cast::channels::media::{IdleReason, Media, PlayerState, StatusEntry, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use tokio::sync::mpsc;

use crate::renderer::{EdgeFilter, RendererEvents};

/// How often the owning thread polls the receiver's status. This bounds command latency (a queued
/// command waits at most one poll) and keeps a position report flowing; see the module docs on why
/// polling is what makes a blocking single-threaded client responsive.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The MIME type we advertise for the LAN stream.
const FLAC_MIME: &str = "audio/flac";

/// Classify one Cast status entry, given the media session we own.
///
/// This is the whole feedback contract as a pure function, so the interesting cases — a takeover
/// arriving as a *different* `media_session_id`, `IdleReason::Finished` vs `Interrupted` — are
/// unit-testable without a device. `None` means "nothing state-changing to report".
#[must_use]
pub fn classify(entry: &StatusEntry, our_session: Option<i32>) -> Option<RendererEvent> {
    // Until our LOAD is accepted nothing on the receiver is ours — including media still playing
    // from a previous session on the same speaker, which a re-selected sink's first poll sees.
    let ours = our_session?;
    // A status for a session that isn't ours means something else owns the receiver now. This is
    // the external-takeover signal tideway never surfaced.
    if entry.media_session_id != ours {
        return Some(RendererEvent::Superseded(format!(
            "another sender owns the receiver (session {} != {ours})",
            entry.media_session_id
        )));
    }

    match entry.player_state {
        PlayerState::Playing => Some(RendererEvent::State(RendererState::Playing)),
        PlayerState::Paused => Some(RendererEvent::State(RendererState::Paused)),
        PlayerState::Buffering => Some(RendererEvent::State(RendererState::Buffering)),
        PlayerState::Idle => match entry.idle_reason {
            Some(IdleReason::Finished | IdleReason::Cancelled) => Some(RendererEvent::Ended),
            Some(IdleReason::Interrupted) => Some(RendererEvent::Superseded(
                "receiver loaded different media".to_string(),
            )),
            Some(IdleReason::Error) => Some(RendererEvent::Failed(
                "receiver reported an error".to_string(),
            )),
            // Idle with no reason = the player just started and has nothing loaded yet.
            None => None,
        },
    }
}

/// The position a status entry reports, if it is one we should believe.
///
/// Only a *playing* entry of *our* media session says anything live: a paused or buffering
/// receiver keeps reporting the same `current_time`, and an entry belonging to someone else's
/// session — or any entry before our LOAD has been accepted — is not our playback at all. Kept
/// pure and separate from [`classify`] because position is a continuous correction, not a state
/// transition.
#[must_use]
pub fn reported_position(entry: &StatusEntry, our_session: Option<i32>) -> Option<Duration> {
    if our_session != Some(entry.media_session_id) {
        return None;
    }
    if entry.player_state != PlayerState::Playing {
        return None;
    }
    Duration::try_from_secs_f32(entry.current_time?).ok()
}

/// What a status with *no* media entry means once we have loaded some: our media session has
/// ended, and the receiver has discarded it.
///
/// That is how the end of a track actually shows up to a polling sender. The receiver broadcasts
/// `IDLE`/`FINISHED` once, unsolicited, at the moment the media ends — between two of our polls,
/// where it is lost — and from then on answers every status request with no entries at all.
/// Waiting for a `FINISHED` we will never see is how the queue failed to advance on a real
/// speaker. Media replaced by *another* sender is not this case: its entry carries a different
/// session id and classifies as a takeover.
#[must_use]
pub fn media_gone(status: &MediaStatus, our_session: Option<i32>) -> Option<RendererEvent> {
    (our_session.is_some() && status.entries.is_empty()).then_some(RendererEvent::Ended)
}

/// Commands the async side sends to the connection-owning thread.
#[derive(Debug)]
enum CastCommand {
    Load { url: String, load: LoadId },
    Play,
    Pause,
    Stop,
    SetVolume(f32),
    SetMuted(bool),
    Shutdown,
}

/// A connected Chromecast receiver, driven over its own I/O thread.
///
/// Dropping it shuts the thread down and disconnects, so a dropped sink never leaves a receiver
/// holding our stream.
pub struct CastSink {
    id: SinkId,
    commands: mpsc::UnboundedSender<CastCommand>,
    /// Source of [`LoadId`]s for this session.
    loads: AtomicU64,
}

impl CastSink {
    /// Connect to a receiver and launch the Default Media Receiver app. Returns the sink and the
    /// stream of what the device reports; the stream closes when the session ends.
    ///
    /// Runs the blocking connect on a worker thread, so it is safe to call from async context.
    ///
    /// # Errors
    /// Returns [`Error::Sink`] if the device is unreachable or the app can't be launched.
    pub async fn connect(id: SinkId, addr: SocketAddr) -> Result<(Self, RendererEvents)> {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (evt_tx, evt_rx) = mpsc::unbounded_channel();
        // The media session the device assigns our LOAD (0 = not loaded yet), kept by the thread.
        let session = Arc::new(AtomicI32::new(0));

        // The thread owns the connection; it reports readiness (or a connect failure) once.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name("canon-cast".into())
            .spawn(move || run_connection(addr, cmd_rx, evt_tx, session, ready_tx))
            .map_err(|e| Error::Sink(format!("spawn cast thread: {e}")))?;

        match ready_rx.await {
            Ok(Ok(())) => Ok((
                Self {
                    id,
                    commands: cmd_tx,
                    loads: AtomicU64::new(0),
                },
                evt_rx,
            )),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::Sink("cast thread died during connect".to_string())),
        }
    }

    fn send(&self, command: CastCommand) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| Error::Sink("cast connection is gone".to_string()))
    }
}

impl Sink for CastSink {
    fn id(&self) -> SinkId {
        self.id.clone()
    }
    fn kind(&self) -> SinkKind {
        SinkKind::Chromecast
    }
    fn load(&self, url: &str, _meta: &TrackMeta) -> Result<LoadId> {
        let load = LoadId(self.loads.fetch_add(1, Ordering::Relaxed) + 1);
        self.send(CastCommand::Load {
            url: url.to_string(),
            load,
        })?;
        Ok(load)
    }
    fn play(&self) -> Result<()> {
        self.send(CastCommand::Play)
    }
    fn pause(&self) -> Result<()> {
        self.send(CastCommand::Pause)
    }
    fn stop(&self) -> Result<()> {
        self.send(CastCommand::Stop)
    }
    fn set_volume(&self, volume: f32) -> Result<()> {
        self.send(CastCommand::SetVolume(volume.clamp(0.0, 1.0)))
    }
    fn set_muted(&self, muted: bool) -> Result<()> {
        self.send(CastCommand::SetMuted(muted))
    }
}

impl Drop for CastSink {
    fn drop(&mut self) {
        // Best-effort: tell the thread to stop the receiver and disconnect. If the channel is
        // already closed the thread is gone, which is the same end state.
        let _ = self.commands.send(CastCommand::Shutdown);
    }
}

/// The connection-owning thread: all Cast I/O happens here (see the module docs on why).
///
/// Returning drops `events`, which closes the stream: that is how the session's end is observed.
/// A failure is reported as [`RendererEvent::Failed`] first, so the reason is not lost.
fn run_connection(
    addr: SocketAddr,
    mut commands: mpsc::UnboundedReceiver<CastCommand>,
    events: mpsc::UnboundedSender<RendererReport>,
    session: Arc<AtomicI32>,
    ready: tokio::sync::oneshot::Sender<Result<()>>,
) {
    let host = addr.ip().to_string();
    // Cast devices present a certificate for their own internal name, which no public root
    // attests; the connection is to a pinned LAN address we just discovered, so host
    // verification cannot succeed and is not what protects us here.
    let device = match CastDevice::connect_without_host_verification(host.clone(), addr.port()) {
        Ok(device) => device,
        Err(e) => {
            let _ = ready.send(Err(Error::Sink(format!("cast connect {addr}: {e}"))));
            return;
        }
    };

    if let Err(e) = device.connection.connect("receiver-0") {
        let _ = ready.send(Err(Error::Sink(format!("cast connect channel: {e}"))));
        return;
    }
    let app = match device
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)
    {
        Ok(app) => app,
        Err(e) => {
            let _ = ready.send(Err(Error::Sink(format!("launch media receiver: {e}"))));
            return;
        }
    };
    if let Err(e) = device.connection.connect(app.transport_id.as_str()) {
        let _ = ready.send(Err(Error::Sink(format!("connect to app: {e}"))));
        return;
    }
    let _ = ready.send(Ok(()));

    let transport = app.transport_id.clone();
    let session_id = app.session_id.clone();
    let mut edges = EdgeFilter::default();
    // The load whose media session the receiver is reporting on. Every report is attributed to it;
    // it moves only once the receiver has accepted the next LOAD, so a status that arrives while
    // that LOAD is still queued is correctly still about the old stream.
    let mut current = LoadId(0);

    loop {
        // 1. Service any pending commands (non-blocking).
        loop {
            match commands.try_recv() {
                // Stop our media and leave, but leave the receiver app running. The Default Media
                // Receiver is shared: a new session on the same speaker (re-selecting it, or
                // switching back to it) is handed the *same* app instance, and stopping it here —
                // this thread's teardown lands after the new session's LOAD — silently killed the
                // new session's media, leaving playback wedged in Loading. The idle app times out
                // on its own, as it does for every other sender.
                Ok(CastCommand::Shutdown) => {
                    let media_session = session.load(Ordering::Acquire);
                    if media_session != 0 {
                        let _ = device.media.stop(transport.as_str(), media_session);
                    }
                    let _ = device.connection.disconnect(transport.as_str());
                    return;
                }
                Ok(command) => {
                    let load = match &command {
                        CastCommand::Load { load, .. } => Some(*load),
                        _ => None,
                    };
                    match dispatch(&device, &transport, &session_id, &session, command) {
                        Ok(()) => {
                            if let Some(load) = load {
                                current = load;
                                edges.reset();
                            }
                        }
                        // A command that fails is a real problem (the connection or the receiver
                        // rejected it); report it rather than silently dropping the intent. A
                        // failed LOAD is about the stream we asked for, not the one still playing.
                        Err(e) => {
                            let _ = events.send(RendererReport {
                                load: load.unwrap_or(current),
                                event: RendererEvent::Failed(e.to_string()),
                            });
                        }
                    }
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                // Every handle dropped: the sink is gone, so tear the session down.
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    let media_session = session.load(Ordering::Acquire);
                    if media_session != 0 {
                        let _ = device.media.stop(transport.as_str(), media_session);
                    }
                    let _ = device.connection.disconnect(transport.as_str());
                    return;
                }
            }
        }

        // 2. One blocking request: poll the receiver's status. This both gives us authoritative
        //    state and buffers any unsolicited frames that arrived meanwhile (see module docs).
        let ours = {
            let id = session.load(Ordering::Acquire);
            (id != 0).then_some(id)
        };
        match device.media.get_status(transport.as_str(), ours) {
            Ok(status) => {
                tracing::trace!(?status, "cast status poll");
                report(&status, ours, current, &events, &mut edges);
            }
            Err(e) => {
                let _ = events.send(RendererReport {
                    load: current,
                    event: RendererEvent::Failed(format!("cast status poll: {e}")),
                });
                return;
            }
        }

        // 3. Keep the connection alive from our side. The device's own PING frames arrive while we
        //    are inside a request and are buffered by `receive_find_map`; `rust_cast` exposes no way
        //    to test that buffer without risking a blocking read, so rather than try to answer
        //    their pings we assert liveness with ours. A dead peer then surfaces as a failed ping
        //    or a failed poll, which is exactly the signal we want.
        if let Err(e) = device.heartbeat.ping() {
            let _ = events.send(RendererReport {
                load: current,
                event: RendererEvent::Failed(format!("cast heartbeat: {e}")),
            });
            return;
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Forward what one status frame says about our session — its classification (through the edge
/// filter) and, separately, its position — attributed to `load`.
fn report(
    status: &MediaStatus,
    ours: Option<i32>,
    load: LoadId,
    events: &mpsc::UnboundedSender<RendererReport>,
    edges: &mut EdgeFilter,
) {
    let send = |event| {
        let _ = events.send(RendererReport { load, event });
    };
    if let Some(event) = media_gone(status, ours)
        && edges.admit(&event)
    {
        send(event);
    }
    for entry in &status.entries {
        if let Some(event) = classify(entry, ours)
            && edges.admit(&event)
        {
            send(event);
        }
        if let Some(position) = reported_position(entry, ours) {
            send(RendererEvent::Position(position));
        }
    }
}

/// Execute one command against the receiver. Runs on the owning thread only.
fn dispatch(
    device: &CastDevice<'_>,
    transport: &str,
    session_id: &str,
    session: &Arc<AtomicI32>,
    command: CastCommand,
) -> std::result::Result<(), Error> {
    let media_session = || -> std::result::Result<i32, Error> {
        let id = session.load(Ordering::Acquire);
        if id == 0 {
            Err(Error::Sink("no media session loaded".to_string()))
        } else {
            Ok(id)
        }
    };

    match command {
        CastCommand::Load { url, .. } => {
            // Buffered, not Live: each load is one track whose stream ends when it has been fed,
            // and only a buffered stream's end reads as the media finishing. A live stream that
            // ends is a stall to the receiver — it sits "playing" into silence, then buffering,
            // and never reports FINISHED, so the queue would never advance.
            let media = Media {
                content_id: url,
                stream_type: StreamType::Buffered,
                content_type: FLAC_MIME.to_string(),
                metadata: None,
                duration: None,
            };
            let status = device
                .media
                .load(transport, session_id, &media)
                .map_err(|e| Error::Sink(format!("cast load: {e}")))?;
            tracing::info!(?status, "cast load accepted");
            if let Some(entry) = status.entries.first() {
                session.store(entry.media_session_id, Ordering::Release);
            }
            Ok(())
        }
        CastCommand::Play => device
            .media
            .play(transport, media_session()?)
            .map(|_| ())
            .map_err(|e| Error::Sink(format!("cast play: {e}"))),
        CastCommand::Pause => device
            .media
            .pause(transport, media_session()?)
            .map(|_| ())
            .map_err(|e| Error::Sink(format!("cast pause: {e}"))),
        CastCommand::Stop => device
            .media
            .stop(transport, media_session()?)
            .map(|_| ())
            .map_err(|e| Error::Sink(format!("cast stop: {e}"))),
        // The receiver answers with its resulting volume, which is the only confirmation a volume
        // change gets (media status does not carry it).
        CastCommand::SetVolume(volume) => device
            .receiver
            .set_volume(volume)
            .map(|v| tracing::debug!(level = ?v.level, muted = ?v.muted, "cast volume set"))
            .map_err(|e| Error::Sink(format!("cast volume: {e}"))),
        CastCommand::SetMuted(muted) => device
            .receiver
            .set_volume(muted)
            .map(|v| tracing::debug!(level = ?v.level, muted = ?v.muted, "cast mute set"))
            .map_err(|e| Error::Sink(format!("cast mute: {e}"))),
        CastCommand::Shutdown => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a status entry in a given player state. `rust_cast`'s `StatusEntry` is a plain struct,
    /// so a test can construct device states directly and assert the classification contract
    /// without a receiver.
    fn entry(
        media_session_id: i32,
        player_state: PlayerState,
        idle_reason: Option<IdleReason>,
    ) -> StatusEntry {
        StatusEntry {
            media_session_id,
            media: None,
            playback_rate: 1.0,
            player_state,
            current_item_id: None,
            loading_item_id: None,
            preloaded_item_id: None,
            idle_reason,
            extended_status: None,
            current_time: None,
            supported_media_commands: 0,
        }
    }

    #[test]
    fn transport_states_map_to_events() {
        let ours = Some(7);
        assert_eq!(
            classify(&entry(7, PlayerState::Playing, None), ours),
            Some(RendererEvent::State(RendererState::Playing))
        );
        assert_eq!(
            classify(&entry(7, PlayerState::Paused, None), ours),
            Some(RendererEvent::State(RendererState::Paused))
        );
        assert_eq!(
            classify(&entry(7, PlayerState::Buffering, None), ours),
            Some(RendererEvent::State(RendererState::Buffering))
        );
    }

    #[test]
    fn finished_and_cancelled_end_the_session() {
        let ours = Some(7);
        assert_eq!(
            classify(
                &entry(7, PlayerState::Idle, Some(IdleReason::Finished)),
                ours
            ),
            Some(RendererEvent::Ended)
        );
        assert_eq!(
            classify(
                &entry(7, PlayerState::Idle, Some(IdleReason::Cancelled)),
                ours
            ),
            Some(RendererEvent::Ended)
        );
    }

    /// The tideway bug this exists to prevent: another sender grabs the receiver and the app keeps
    /// asserting stale control. A status for a different media session must surface as a takeover.
    #[test]
    fn a_different_media_session_is_a_takeover() {
        let event = classify(&entry(99, PlayerState::Playing, None), Some(7));
        assert!(
            matches!(event, Some(RendererEvent::Superseded(_))),
            "a foreign media session must surface as a takeover, got {event:?}"
        );
    }

    #[test]
    fn interrupted_is_a_takeover_not_an_end() {
        let event = classify(
            &entry(7, PlayerState::Idle, Some(IdleReason::Interrupted)),
            Some(7),
        );
        assert!(matches!(event, Some(RendererEvent::Superseded(_))));
    }

    #[test]
    fn receiver_error_fails() {
        let event = classify(
            &entry(7, PlayerState::Idle, Some(IdleReason::Error)),
            Some(7),
        );
        assert!(matches!(event, Some(RendererEvent::Failed(_))));
    }

    /// A condition keeps being reported for as long as it holds. Suppressing the repeats here
    /// once wedged playback: a re-LOAD puts the player back to `Loading`, and with the
    /// receiver's `Playing` swallowed as "unchanged", nothing ever told it otherwise.
    #[test]
    fn a_repeated_condition_keeps_being_reported() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut edges = EdgeFilter::default();
        let status = MediaStatus {
            request_id: 1,
            entries: vec![entry(7, PlayerState::Playing, None)],
        };

        for _ in 0..5 {
            report(&status, Some(7), LoadId(1), &tx, &mut edges);
        }
        for _ in 0..5 {
            assert_eq!(
                rx.try_recv().map(|report| report.event),
                Ok(RendererEvent::State(RendererState::Playing))
            );
        }

        let paused = MediaStatus {
            request_id: 2,
            entries: vec![entry(7, PlayerState::Paused, None)],
        };
        report(&paused, Some(7), LoadId(1), &tx, &mut edges);
        assert_eq!(
            rx.try_recv().map(|report| report.event),
            Ok(RendererEvent::State(RendererState::Paused))
        );
    }

    /// Edges are the other half of the contract: each drives a one-shot action (queue
    /// auto-advance, fail-back), so the continuous poll must not re-fire them.
    #[test]
    fn a_terminal_event_fires_once() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut edges = EdgeFilter::default();
        let finished = MediaStatus {
            request_id: 1,
            entries: vec![entry(7, PlayerState::Idle, Some(IdleReason::Finished))],
        };

        for _ in 0..5 {
            report(&finished, Some(7), LoadId(1), &tx, &mut edges);
        }
        assert_eq!(
            rx.try_recv().map(|report| report.event),
            Ok(RendererEvent::Ended)
        );
        assert!(
            rx.try_recv().map(|report| report.event).is_err(),
            "a finished stream must not advance the queue five times"
        );
    }

    fn playing_at(media_session_id: i32, secs: f32) -> StatusEntry {
        StatusEntry {
            current_time: Some(secs),
            ..entry(media_session_id, PlayerState::Playing, None)
        }
    }

    #[test]
    fn a_playing_entry_reports_its_position() {
        assert_eq!(
            reported_position(&playing_at(7, 76.5), Some(7)),
            Some(Duration::from_millis(76_500))
        );
    }

    #[test]
    fn position_from_someone_elses_session_is_not_ours() {
        assert_eq!(reported_position(&playing_at(9, 76.5), Some(7)), None);
    }

    #[test]
    fn only_a_playing_entry_reports_a_live_position() {
        // A paused receiver keeps repeating its last position; believing it as "live" would
        // have us reconcile against a value that is no longer moving.
        let paused = StatusEntry {
            current_time: Some(76.5),
            ..entry(7, PlayerState::Paused, None)
        };
        assert_eq!(reported_position(&paused, Some(7)), None);
    }

    /// Position is a continuous correction, so unlike state it must arrive on every poll.
    #[test]
    fn every_poll_reports_position_even_when_the_state_is_unchanged() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut edges = EdgeFilter::default();

        for tick in 0..3 {
            let status = MediaStatus {
                request_id: 1,
                entries: vec![playing_at(7, 10.0 + f32::from(tick as u8))],
            };
            report(&status, Some(7), LoadId(1), &tx, &mut edges);
        }

        let mut positions = Vec::new();
        while let Ok(event) = rx.try_recv().map(|report| report.event) {
            if let RendererEvent::Position(position) = event {
                positions.push(position);
            }
        }
        assert_eq!(
            positions,
            vec![
                Duration::from_secs(10),
                Duration::from_secs(11),
                Duration::from_secs(12)
            ],
            "every poll's position must reach the player"
        );
    }

    /// The end of a track, as a polling sender sees it: the receiver has dropped our media
    /// session, and answers with no entries at all.
    #[test]
    fn our_media_vanishing_is_the_end_of_the_track() {
        let empty = MediaStatus {
            request_id: 1,
            entries: vec![],
        };
        assert_eq!(media_gone(&empty, Some(7)), Some(RendererEvent::Ended));
        assert_eq!(
            media_gone(&empty, None),
            None,
            "before our LOAD there was never any media of ours to lose"
        );
        let playing = MediaStatus {
            request_id: 2,
            entries: vec![entry(7, PlayerState::Playing, None)],
        };
        assert_eq!(media_gone(&playing, Some(7)), None);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut edges = EdgeFilter::default();
        for _ in 0..5 {
            report(&empty, Some(7), LoadId(1), &tx, &mut edges);
        }
        assert_eq!(
            rx.try_recv().map(|report| report.event),
            Ok(RendererEvent::Ended)
        );
        assert!(rx.try_recv().is_err(), "every later poll is the same end");
    }

    /// Idle with no reason is the receiver's "nothing loaded yet" resting state, not an event.
    #[test]
    fn bare_idle_reports_nothing() {
        assert_eq!(classify(&entry(7, PlayerState::Idle, None), Some(7)), None);
        // Before our LOAD lands we have no session, so any status is informational only.
        assert_eq!(
            classify(&entry(0, PlayerState::Idle, None), None),
            None,
            "no session yet + bare idle is not an event"
        );
    }

    /// Re-selecting a speaker opens a new session on the *same* receiver app, whose first poll
    /// sees the previous session's media still playing. That is not us: reporting it flipped the
    /// player to Playing before our stream had even been loaded, and fed it a foreign position.
    #[test]
    fn nothing_is_ours_before_our_load_is_accepted() {
        assert_eq!(classify(&playing_at(1, 7.0), None), None);
        assert_eq!(reported_position(&playing_at(1, 7.0), None), None);
    }
}
