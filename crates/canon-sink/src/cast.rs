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
//! `MEDIA_STATUS` is the authority: every status frame is translated into a [`CastEvent`] and fed
//! back through the player state actor as an ordinary input, exactly like a local device change.
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
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

use canon_core::{Error, Result, SinkHealth, SinkId, SinkKind};
use rust_cast::CastDevice;
use rust_cast::channels::media::Status as MediaStatus;
use rust_cast::channels::media::{IdleReason, Media, PlayerState, StatusEntry, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use tokio::sync::{mpsc, watch};

/// How often the owning thread polls the receiver's status. This bounds command latency (a queued
/// command waits at most one poll) and keeps a position report flowing; see the module docs on why
/// polling is what makes a blocking single-threaded client responsive.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// The MIME type we advertise for the LAN stream.
const FLAC_MIME: &str = "audio/flac";

/// What the *device* told us. These are inputs to the player state machine, not confirmations of
/// our own commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastEvent {
    /// The receiver is playing our stream.
    Playing,
    /// The receiver paused (possibly from the device itself or another sender).
    Paused,
    /// The receiver is buffering our stream.
    Buffering,
    /// Our media session ended normally (stream exhausted / stopped by us).
    Ended,
    /// Our session was taken over or evicted — another sender loaded media, or the receiver
    /// cancelled us. The player must stop asserting control and fail back.
    Superseded(String),
    /// The receiver reported an error, or the connection died.
    Failed(String),
}

/// Translate one Cast status entry into a [`CastEvent`], given the media session we own.
///
/// This is the whole feedback contract as a pure function, so the interesting cases — a takeover
/// arriving as a *different* `media_session_id`, `IdleReason::Finished` vs `Interrupted` — are
/// unit-testable without a device. `None` means "nothing state-changing to report".
#[must_use]
pub fn classify(entry: &StatusEntry, our_session: Option<i32>) -> Option<CastEvent> {
    // A status for a session that isn't ours means something else owns the receiver now. This is
    // the external-takeover signal tideway never surfaced.
    if let Some(ours) = our_session
        && entry.media_session_id != ours
    {
        return Some(CastEvent::Superseded(format!(
            "another sender owns the receiver (session {} != {ours})",
            entry.media_session_id
        )));
    }

    match entry.player_state {
        PlayerState::Playing => Some(CastEvent::Playing),
        PlayerState::Paused => Some(CastEvent::Paused),
        PlayerState::Buffering => Some(CastEvent::Buffering),
        PlayerState::Idle => match entry.idle_reason {
            Some(IdleReason::Finished) => Some(CastEvent::Ended),
            Some(IdleReason::Cancelled) => Some(CastEvent::Ended),
            Some(IdleReason::Interrupted) => Some(CastEvent::Superseded(
                "receiver loaded different media".to_string(),
            )),
            Some(IdleReason::Error) => {
                Some(CastEvent::Failed("receiver reported an error".to_string()))
            }
            // Idle with no reason = the player just started and has nothing loaded yet.
            None => None,
        },
    }
}

/// Commands the async side sends to the connection-owning thread.
#[derive(Debug)]
enum CastCommand {
    Load { url: String },
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    Shutdown,
}

/// A connected Chromecast receiver, driven over its own I/O thread.
///
/// Dropping it shuts the thread down and disconnects, so a dropped sink never leaves a receiver
/// holding our stream.
pub struct CastSink {
    id: SinkId,
    name: String,
    commands: mpsc::UnboundedSender<CastCommand>,
    events: mpsc::UnboundedReceiver<CastEvent>,
    health: watch::Receiver<SinkHealth>,
    /// The media session the device assigned our LOAD, published by the I/O thread so the async
    /// side can report it (0 = not loaded yet).
    session: Arc<AtomicI32>,
}

impl CastSink {
    /// Connect to a receiver and launch the Default Media Receiver app.
    ///
    /// Runs the blocking connect on a worker thread, so it is safe to call from async context.
    ///
    /// # Errors
    /// Returns [`Error::Sink`] if the device is unreachable or the app can't be launched.
    pub async fn connect(id: SinkId, name: String, addr: SocketAddr) -> Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (evt_tx, evt_rx) = mpsc::unbounded_channel();
        let (health_tx, health_rx) = watch::channel(SinkHealth::Healthy);
        let session = Arc::new(AtomicI32::new(0));

        // The thread owns the connection; it reports readiness (or a connect failure) once.
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let thread_session = Arc::clone(&session);
        std::thread::Builder::new()
            .name("canon-cast".into())
            .spawn(move || {
                run_connection(addr, cmd_rx, evt_tx, health_tx, thread_session, ready_tx);
            })
            .map_err(|e| Error::Sink(format!("spawn cast thread: {e}")))?;

        match ready_rx.await {
            Ok(Ok(())) => Ok(Self {
                id,
                name,
                commands: cmd_tx,
                events: evt_rx,
                health: health_rx,
                session,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::Sink("cast thread died during connect".to_string())),
        }
    }

    /// The device's friendly name, as discovered.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The media session id the receiver assigned our stream, if loaded.
    #[must_use]
    pub fn media_session(&self) -> Option<i32> {
        let id = self.session.load(Ordering::Acquire);
        (id != 0).then_some(id)
    }

    /// Point the receiver at a stream URL (our LAN FLAC server) and start it.
    pub fn load(&self, url: String) -> Result<()> {
        self.send(CastCommand::Load { url })
    }

    /// Take the next device-reported event, if one has arrived.
    pub async fn next_event(&mut self) -> Option<CastEvent> {
        self.events.recv().await
    }

    fn send(&self, command: CastCommand) -> Result<()> {
        self.commands
            .send(command)
            .map_err(|_| Error::Sink("cast connection is gone".to_string()))
    }
}

impl CastSink {
    pub fn id(&self) -> SinkId {
        self.id.clone()
    }
    pub fn kind(&self) -> SinkKind {
        SinkKind::Chromecast
    }
    pub fn play(&self) -> Result<()> {
        self.send(CastCommand::Play)
    }
    pub fn pause(&self) -> Result<()> {
        self.send(CastCommand::Pause)
    }
    pub fn stop(&self) -> Result<()> {
        self.send(CastCommand::Stop)
    }
    pub fn seek(&self, position: Duration) -> Result<()> {
        self.send(CastCommand::Seek(position))
    }
    pub fn set_volume(&self, volume: f32) -> Result<()> {
        self.send(CastCommand::SetVolume(volume))
    }
    /// Live liveness view; the player watches this and fails back to local on `Failed`.
    pub fn health(&self) -> watch::Receiver<SinkHealth> {
        self.health.clone()
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
fn run_connection(
    addr: SocketAddr,
    mut commands: mpsc::UnboundedReceiver<CastCommand>,
    events: mpsc::UnboundedSender<CastEvent>,
    health: watch::Sender<SinkHealth>,
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
    // The last state reported, so a steady state isn't re-sent on every poll.
    let mut last_reported: Option<CastEvent> = None;

    loop {
        // 1. Service any pending commands (non-blocking).
        loop {
            match commands.try_recv() {
                Ok(CastCommand::Shutdown) => {
                    let media_session = session.load(Ordering::Acquire);
                    if media_session != 0 {
                        let _ = device.media.stop(transport.as_str(), media_session);
                    }
                    let _ = device.receiver.stop_app(session_id.as_str());
                    let _ = device.connection.disconnect(transport.as_str());
                    return;
                }
                Ok(command) => {
                    if let Err(e) = dispatch(&device, &transport, &session_id, &session, command) {
                        // A command that fails is a real problem (the connection or the receiver
                        // rejected it); report it rather than silently dropping the intent.
                        let _ = events.send(CastEvent::Failed(e.to_string()));
                        let _ = health.send(SinkHealth::Degraded(e.to_string()));
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
                if *health.borrow() != SinkHealth::Healthy {
                    let _ = health.send(SinkHealth::Healthy);
                }
                report(&status, &session, &events, &mut last_reported);
            }
            Err(e) => {
                let why = format!("cast status poll: {e}");
                let _ = health.send(SinkHealth::Failed(why.clone()));
                let _ = events.send(CastEvent::Failed(why));
                return;
            }
        }

        // 3. Keep the connection alive from our side. The device's own PING frames arrive while we
        //    are inside a request and are buffered by `receive_find_map`; `rust_cast` exposes no way
        //    to test that buffer without risking a blocking read, so rather than try to answer
        //    their pings we assert liveness with ours. A dead peer then surfaces as a failed ping
        //    or a failed poll, which is exactly the signal we want.
        if let Err(e) = device.heartbeat.ping() {
            let why = format!("cast heartbeat: {e}");
            let _ = health.send(SinkHealth::Failed(why.clone()));
            let _ = events.send(CastEvent::Failed(why));
            return;
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Classify every entry of a status frame and forward only *transitions*.
///
/// The status poll runs continuously, so an unchanged state arrives every interval. Forwarding
/// those verbatim would spam the state machine with redundant inputs (and, since each one is a
/// transition as far as the player is concerned, churn the emitted snapshot). `last` carries the
/// previously reported event so a steady state is reported once; terminal events still pass
/// through so the caller can act on them.
fn report(
    status: &MediaStatus,
    session: &Arc<AtomicI32>,
    events: &mpsc::UnboundedSender<CastEvent>,
    last: &mut Option<CastEvent>,
) {
    let ours = {
        let id = session.load(Ordering::Acquire);
        (id != 0).then_some(id)
    };
    for entry in &status.entries {
        if let Some(event) = classify(entry, ours) {
            if last.as_ref() == Some(&event) {
                continue; // same state as last poll; nothing changed to report
            }
            *last = Some(event.clone());
            let _ = events.send(event);
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
        CastCommand::Load { url } => {
            let media = Media {
                content_id: url,
                stream_type: StreamType::Live,
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
        CastCommand::Seek(position) => device
            .media
            .seek(
                transport,
                media_session()?,
                Some(position.as_secs_f32()),
                None,
            )
            .map(|_| ())
            .map_err(|e| Error::Sink(format!("cast seek: {e}"))),
        CastCommand::SetVolume(volume) => device
            .receiver
            .set_volume(volume)
            .map(|_| ())
            .map_err(|e| Error::Sink(format!("cast volume: {e}"))),
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
            Some(CastEvent::Playing)
        );
        assert_eq!(
            classify(&entry(7, PlayerState::Paused, None), ours),
            Some(CastEvent::Paused)
        );
        assert_eq!(
            classify(&entry(7, PlayerState::Buffering, None), ours),
            Some(CastEvent::Buffering)
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
            Some(CastEvent::Ended)
        );
        assert_eq!(
            classify(
                &entry(7, PlayerState::Idle, Some(IdleReason::Cancelled)),
                ours
            ),
            Some(CastEvent::Ended)
        );
    }

    /// The tideway bug this exists to prevent: another sender grabs the receiver and the app keeps
    /// asserting stale control. A status for a different media session must surface as a takeover.
    #[test]
    fn a_different_media_session_is_a_takeover() {
        let event = classify(&entry(99, PlayerState::Playing, None), Some(7));
        assert!(
            matches!(event, Some(CastEvent::Superseded(_))),
            "a foreign media session must surface as a takeover, got {event:?}"
        );
    }

    #[test]
    fn interrupted_is_a_takeover_not_an_end() {
        let event = classify(
            &entry(7, PlayerState::Idle, Some(IdleReason::Interrupted)),
            Some(7),
        );
        assert!(matches!(event, Some(CastEvent::Superseded(_))));
    }

    #[test]
    fn receiver_error_fails() {
        let event = classify(
            &entry(7, PlayerState::Idle, Some(IdleReason::Error)),
            Some(7),
        );
        assert!(matches!(event, Some(CastEvent::Failed(_))));
    }

    /// The status poll fires continuously, so an unchanged state must be reported once rather than
    /// re-sent every interval (observed live: 54 identical "playing" events in ~27s before this).
    #[test]
    fn a_steady_state_is_reported_once() {
        let session = Arc::new(AtomicI32::new(7));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut last = None;
        let status = MediaStatus {
            request_id: 1,
            entries: vec![entry(7, PlayerState::Playing, None)],
        };

        for _ in 0..5 {
            report(&status, &session, &tx, &mut last);
        }
        assert_eq!(rx.try_recv(), Ok(CastEvent::Playing));
        assert!(
            rx.try_recv().is_err(),
            "an unchanged state must not be re-reported on every poll"
        );

        // A real transition still gets through.
        let paused = MediaStatus {
            request_id: 2,
            entries: vec![entry(7, PlayerState::Paused, None)],
        };
        report(&paused, &session, &tx, &mut last);
        assert_eq!(rx.try_recv(), Ok(CastEvent::Paused));
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
}
