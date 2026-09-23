//! The playback controller (yaks canon-a7d6 + canon-23f5): the daemon-level glue that
//! turns a [`Command`] into actual sound, owns the play queue, and keeps the player state
//! actor authoritative.
//!
//! It implements [`ControlPlane`], so `canon-api` drives it exactly like the bare player.
//! Beyond a single `Load`, it holds a **server-owned queue** (the queue lives here, not in
//! any client) with next/previous and **auto-advance** on end-of-track: when the audio
//! engine reports [`EngineEvent::Ended`], the controller starts the next queued track
//! rather than passing Ended straight through.
//!
//! Concurrency: the queue + current playback live behind one async mutex, and every
//! playback start bumps a **generation** counter. Long work (metadata + stream resolve)
//! runs without the lock held, then the freshly-resolved engine is installed only if its
//! generation is still current — so a client `Next` racing an auto-advance can't double-
//! skip or install a stale stream.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use canon_audio::{AudioPlayer, Output};
use canon_core::{
    Codec, Command, ControlPlane, EngineEvent, Error, LoadId, PcmSink, PlayerHandle,
    PlayerSnapshot, Quality, QueueView, RendererEvent, RendererReport, Result, Sink, SinkId,
    SinkInfo, Source, SourceRef, TrackRef,
};
use canon_sink::{DiscoveryService, FlacTap, RendererEvents, StreamRoutes};
use canon_tidal::TidalSession;
use tokio::sync::{Mutex, Notify, watch};

/// Queue + current-playback state, guarded by one mutex.
#[derive(Default)]
struct Inner {
    queue: Vec<TrackRef>,
    /// Index of the current track within `queue` (meaningful while `active`).
    index: usize,
    /// True once a track is loaded/playing; false when idle, stopped, or the queue is
    /// exhausted. Gates auto-start on enqueue.
    active: bool,
    /// Bumped on every playback start; the async resolve installs its engine only if this
    /// still matches, so superseded starts are discarded.
    generation: u64,
    audio: Option<AudioPlayer>,
    /// The selected output. `None` is local; `Some` is a live network session.
    ///
    /// One active output at a time (the canon-dde4 decision): selecting a sink restarts the
    /// current track on it, and dropping this restores local. A headless box with no audio
    /// device must be able to play to a renderer, so "local, muted" is not the model.
    network: Option<NetworkSession>,
}

/// A live network output: the renderer's control session plus the LAN stream server feeding it.
/// Protocol-neutral — the renderer is whatever [`canon_sink::connect`] opened.
///
/// Dropping it drops the sink (which stops the renderer and disconnects) and aborts the stream
/// server task, so teardown needs no separate cleanup step to forget.
struct NetworkSession {
    sink: Box<dyn Sink>,
    /// Which session this is. The tasks that watch a session (its renderer events, the stream
    /// watchdog) carry this rather than the sink id, so re-selecting the *same* speaker can't let
    /// the old session's late teardown be mistaken for the new one failing.
    epoch: u64,
    /// This session's streams: every load gets a fresh one, whose body ends once its track has
    /// been fed, so the renderer can finish the track and the queue can advance.
    routes: StreamRoutes,
    /// Where the stream server is reachable from the renderer (`http://ip:port`).
    base_url: String,
    /// The speaker's address: what identifies it across protocols (see `canon_sink::outputs`).
    host: IpAddr,
    /// The load the renderer is playing for us, and the playback generation it was issued for.
    /// `None` before the first load and after a stop: then nothing the renderer says about its
    /// media is about ours.
    current: Option<(LoadId, u64)>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for NetworkSession {
    fn drop(&mut self) {
        self.server.abort();
    }
}

type TaggedEvent = (u64, EngineEvent);

pub struct PlaybackController {
    player: PlayerHandle,
    session: Arc<TidalSession>,
    quality: Quality,
    inner: Mutex<Inner>,
    /// Engine events from every playback, tagged with their generation, funnel here to a
    /// single processor task. This one-channel indirection is also what keeps the
    /// play_index / on_engine_event recursion from forming a non-`Send` future cycle.
    engine_tx: tokio::sync::mpsc::UnboundedSender<TaggedEvent>,
    /// The client-facing snapshot stream: the player's state merged with the queue view.
    snapshots: watch::Sender<PlayerSnapshot>,
    /// Nudged on queue changes that don't move player state (e.g. enqueue while playing),
    /// so the merged snapshot refreshes promptly.
    dirty: Arc<Notify>,
    /// LAN renderer discovery, shared with the API's sink listing. `None` when discovery could not
    /// start (e.g. no permission on macOS): local playback still works, and selecting a network
    /// sink reports that discovery is unavailable rather than failing obscurely.
    discovery: Option<Arc<DiscoveryService>>,
    /// Source of [`NetworkSession::epoch`]s.
    sessions: AtomicU64,
    me: std::sync::Weak<Self>,
}

/// Bit depth for the network FLAC stream. 16-bit is what every renderer accepts.
const FLAC_BITS: u16 = 16;
/// How often the stream watchdog checks whether a renderer is still pulling our bytes.
const CONSUMER_POLL: Duration = Duration::from_secs(1);
/// How long the stream may go unconsumed before we conclude the session is gone. Generous enough
/// to cover a renderer's initial fetch after LOAD and a brief reconnect (which header replay is
/// designed to serve), short enough that a takeover surfaces promptly.
const CONSUMER_GRACE: Duration = Duration::from_secs(10);

impl PlaybackController {
    pub fn new(
        player: PlayerHandle,
        session: Arc<TidalSession>,
        quality: Quality,
        discovery: Option<Arc<DiscoveryService>>,
    ) -> Arc<Self> {
        let (engine_tx, mut engine_rx) = tokio::sync::mpsc::unbounded_channel::<TaggedEvent>();
        let (snapshots, _) = watch::channel(player.snapshot());
        let controller = Arc::new_cyclic(|me| Self {
            player,
            session,
            quality,
            inner: Mutex::new(Inner::default()),
            engine_tx,
            snapshots,
            dirty: Arc::new(Notify::new()),
            discovery,
            sessions: AtomicU64::new(0),
            me: me.clone(),
        });
        // Single processor: serializes auto-advance and state forwarding.
        let processor = Arc::clone(&controller);
        tokio::spawn(async move {
            while let Some((generation, event)) = engine_rx.recv().await {
                processor.on_engine_event(generation, event).await;
            }
        });
        // Publisher: merge player state + queue view into the client-facing stream.
        let publisher = Arc::clone(&controller);
        tokio::spawn(async move { publisher.run_snapshot_publisher().await });
        controller
    }

    /// Republish the merged snapshot whenever player state changes or the queue is nudged.
    async fn run_snapshot_publisher(&self) {
        let mut player_rx = self.player.subscribe();
        loop {
            let mut snapshot = self.player.snapshot();
            snapshot.queue = self.queue_view().await;
            let _ = self.snapshots.send_replace(snapshot);
            tokio::select! {
                changed = player_rx.changed() => {
                    if changed.is_err() {
                        break; // player actor gone
                    }
                }
                () = self.dirty.notified() => {}
            }
        }
    }

    async fn queue_view(&self) -> Option<QueueView> {
        let inner = self.inner.lock().await;
        (!inner.queue.is_empty()).then(|| QueueView {
            len: inner.queue.len(),
            index: inner.index,
        })
    }

    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("controller alive")
    }

    /// Start playing `queue[index]` from the beginning.
    async fn play_index(&self, index: usize) -> Result<()> {
        self.play_index_at(index, std::time::Duration::ZERO).await
    }

    /// Start playing `queue[index]` from `position`. Bumps the generation, stops any
    /// current engine, then resolves and installs the new engine off-lock (discarding the
    /// result if a newer start superseded this one).
    async fn play_index_at(&self, index: usize, position: std::time::Duration) -> Result<()> {
        let (mut track, generation) = {
            let mut inner = self.inner.lock().await;
            if index >= inner.queue.len() {
                return Ok(());
            }
            inner.index = index;
            inner.active = true;
            inner.generation += 1;
            if let Some(previous) = inner.audio.take() {
                previous.stop();
            }
            (inner.queue[index].clone(), inner.generation)
        };

        let Some(id) = tidal_id(&track) else {
            let message = "track has no Tidal source".to_string();
            self.player.command(Command::Load(track)).await;
            self.player
                .engine(EngineEvent::Failed(message.clone()))
                .await;
            return Err(Error::Unsupported(message));
        };

        // Fill display metadata (title/artist/duration) so the snapshot carries a real
        // duration the moment we go to Loading.
        if track.meta.duration_ms.is_none() {
            let source = SourceRef::Tidal { id: id.clone() };
            if let Ok(meta) = Source::track_meta(&*self.session, &source).await {
                track.meta = meta;
            }
        }
        let meta = track.meta.clone();
        self.player.command(Command::Load(track)).await;

        let (resolved, start_ms) = match self
            .session
            .clone()
            .open_stream_at(&id, self.quality, position)
            .await
        {
            Ok(resolved) => resolved,
            Err(e) => {
                self.player.engine(EngineEvent::Failed(e.to_string())).await;
                return Err(e);
            }
        };

        let hint = codec_hint(resolved.info.codec).map(str::to_owned);
        let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel::<EngineEvent>();

        let mut inner = self.inner.lock().await;
        if inner.generation != generation {
            // A newer start superseded us while resolving; drop this stream silently.
            return Ok(());
        }

        // Tag this playback's events with its generation and funnel them to the single
        // processor task (see `engine_tx`). This forwarder holds no controller reference,
        // which is what keeps the futures `Send`.
        let engine_tx = self.engine_tx.clone();
        tokio::spawn(async move {
            while let Some(event) = events_rx.recv().await {
                if engine_tx.send((generation, event)).is_err() {
                    break;
                }
            }
        });

        // Route to the selected output. A live network session gets a fresh stream for this track,
        // fed by a fresh FLAC tap (a new track means a new STREAMINFO header), and the renderer is
        // told to load it; otherwise we play locally.
        let output = match inner.network.as_mut() {
            Some(session) => {
                let (path, broadcaster) = session.routes.open();
                let tap = FlacTap::new(
                    broadcaster,
                    resolved.info.sample_rate,
                    resolved.info.channels,
                    FLAC_BITS,
                )?;
                let url = format!("{}{path}", session.base_url);
                let load = session.sink.load(&url, &meta)?;
                session.current = Some((load, generation));
                Output::Network(Box::new(tap) as Box<dyn PcmSink>)
            }
            None => Output::Local,
        };

        let audio = AudioPlayer::start(
            resolved.input,
            hint,
            self.player.clock(),
            events_tx,
            start_ms,
            output,
        );
        inner.audio = Some(audio);
        Ok(())
    }

    /// Seek the current track to `position` (segment-granular).
    async fn seek(&self, position: std::time::Duration) -> Result<()> {
        let index = {
            let inner = self.inner.lock().await;
            if !inner.active || inner.queue.is_empty() {
                return Err(Error::Unsupported("nothing to seek".into()));
            }
            inner.index
        };
        self.play_index_at(index, position).await
    }

    /// Switch the active output, restarting the current track on it at the current position.
    ///
    /// Selecting `local` tears down any network session; selecting a discovered renderer connects to
    /// it and stands up a LAN stream server bound to the interface discovery chose. One output is
    /// active at a time, so this is a restart rather than a re-route of a live stream.
    async fn select_sink(&self, id: &SinkId) -> Result<()> {
        let resume_at = std::time::Duration::from_millis(self.player.snapshot().position_ms);

        if SinkInfo::is_local(id) {
            let had_network = {
                let mut inner = self.inner.lock().await;
                inner.network.take().is_some() // Drop ends the renderer session.
            };
            self.player.command(Command::SelectSink(id.clone())).await;
            if had_network {
                self.restart_current(resume_at).await;
            }
            return Ok(());
        }

        let device = self.find_device(id)?;
        // A speaker plays one input at a time. Switching protocols on the *same* speaker (Tunes
        // over Cast to Tunes over DLNA, or back) must release it first: asking it to launch Cast
        // while it is playing DLNA is refused outright, and the attempt still knocks the DLNA
        // playback over. Between different devices the old session stays up until the new one is,
        // so there is no gap.
        let same_speaker = {
            let mut inner = self.inner.lock().await;
            let same = inner
                .network
                .as_ref()
                .is_some_and(|session| session.host == device.addr.ip());
            if same {
                inner.network.take(); // Drop ends the renderer session.
            }
            same
        };
        let session = match self.open_network(&device).await {
            Ok(session) => session,
            Err(e) => {
                // With the speaker already released there is nothing left to go back to; carry on
                // locally rather than leave playback nowhere.
                if same_speaker {
                    self.player
                        .command(Command::SelectSink(SinkInfo::local().id))
                        .await;
                    self.restart_current(resume_at).await;
                }
                return Err(e);
            }
        };
        {
            let mut inner = self.inner.lock().await;
            // Install the new session before dropping the old one, so the old renderer's teardown
            // can't be mistaken for the current output going away.
            let previous = inner.network.replace(session);
            drop(previous);
        }
        self.player.command(Command::SelectSink(id.clone())).await;
        self.restart_current(resume_at).await;
        Ok(())
    }

    /// Look up a discovered device by sink id.
    fn find_device(&self, id: &SinkId) -> Result<canon_sink::DiscoveredDevice> {
        let discovery = self
            .discovery
            .as_ref()
            .ok_or_else(|| Error::Unsupported("renderer discovery is unavailable".into()))?;
        discovery
            .devices()
            .borrow()
            .iter()
            .find(|device| &device.id == id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("no renderer {id:?} discovered")))
    }

    /// Connect to a renderer and stand up the LAN stream server it will pull from.
    async fn open_network(&self, device: &canon_sink::DiscoveredDevice) -> Result<NetworkSession> {
        let lan_ip = lan_ip()?;
        let routes = StreamRoutes::default();
        let (bound, server) = canon_sink::spawn(SocketAddr::new(lan_ip, 0), routes.clone()).await?;
        let base_url = format!("http://{bound}");

        let (sink, events) = connect_settled(device).await?;
        let epoch = self.sessions.fetch_add(1, Ordering::Relaxed) + 1;
        // Fold the device's own reports into the player's state machine (see
        // `run_renderer_events`), and watch whether our bytes are actually being consumed (see
        // `run_stream_watchdog`).
        {
            let controller = self.arc();
            let id = device.id.clone();
            tokio::spawn(async move { controller.run_renderer_events(epoch, id, events).await });
        }
        {
            let controller = self.arc();
            let id = device.id.clone();
            let routes = routes.clone();
            tokio::spawn(async move {
                controller.run_stream_watchdog(epoch, id, routes).await;
            });
        }

        Ok(NetworkSession {
            sink,
            epoch,
            routes,
            base_url,
            host: device.addr.ip(),
            current: None,
            server,
        })
    }

    /// Whether `epoch` names the network session that is live right now.
    async fn is_current(&self, epoch: u64) -> bool {
        self.inner
            .lock()
            .await
            .network
            .as_ref()
            .is_some_and(|session| session.epoch == epoch)
    }

    /// Restart the current track on the newly selected output, resuming at `position`.
    async fn restart_current(&self, position: std::time::Duration) {
        let index = {
            let inner = self.inner.lock().await;
            (inner.active && !inner.queue.is_empty()).then_some(inner.index)
        };
        if let Some(index) = index {
            let _ = self.play_index_at(index, position).await;
        }
    }

    /// Fold one renderer session's reports into the player state machine.
    ///
    /// This is the tideway lesson made structural: the renderer's own view of playback — including
    /// a phone taking the speaker over — arrives as an ordinary state input rather than a side
    /// channel, so the published snapshot can't diverge from what the speaker is doing. A failed
    /// or superseded session falls back to local so playback is never left wedged.
    async fn run_renderer_events(&self, epoch: u64, id: SinkId, mut events: RendererEvents) {
        while let Some(RendererReport { load, event }) = events.recv().await {
            let generation = {
                let inner = self.inner.lock().await;
                // A session we have already switched away from may still be draining reports; the
                // speaker it describes is no longer our output, so they are not news to the player.
                let Some(session) = inner.network.as_ref().filter(|s| s.epoch == epoch) else {
                    return;
                };
                attribute(session.current, inner.generation, load)
            };
            match event {
                // Device reports enter as engine events, never as commands: a command is user
                // intent, and the state machine must be able to tell "the speaker is playing" from
                // "someone pressed play". They describe one stream, so they count only while that
                // stream is the one we are playing.
                RendererEvent::State(state) => {
                    if generation.is_some() {
                        self.player.engine(EngineEvent::RendererState(state)).await;
                    }
                }
                RendererEvent::Position(position) => {
                    if generation.is_some() {
                        self.player
                            .engine(EngineEvent::RendererPosition(position))
                            .await;
                    }
                }
                // Route through the same path as a local end-of-track so the queue auto-advances
                // identically on either output — under the generation of the load that ended, so a
                // track the user already skipped past can't advance the queue a second time.
                RendererEvent::Ended => {
                    if let Some(generation) = generation {
                        self.on_engine_event(generation, EngineEvent::Ended).await;
                    }
                }
                // Losing the renderer is about the session, not one stream: honoured whatever load
                // is current (a takeover mid-track-change must not be dropped).
                RendererEvent::Superseded(why) | RendererEvent::Failed(why) => {
                    tracing::warn!(sink = ?id, "renderer session lost: {why}");
                    self.fail_back_to_local(epoch, &id).await;
                    return;
                }
            }
        }
        // The stream closed without saying why. If we dropped the session this is a no-op; if the
        // renderer's connection vanished under us, local takes over.
        self.fail_back_to_local(epoch, &id).await;
    }

    /// Watch whether the renderer is still *consuming* our stream, and fail back if it stops.
    ///
    /// The Cast control channel only knows about Cast. A multi-protocol speaker can be taken over
    /// by something it cannot see — observed live: a KEF playing our stream was grabbed over
    /// Spotify Connect while every Cast status poll still reported our own session `Playing`, so
    /// canon kept claiming it was casting while the room was playing something else (yak
    /// canon-2dbf, and precisely the tideway desync this family exists to prevent).
    ///
    /// Whether anything is pulling bytes from the stream server is protocol-agnostic ground truth,
    /// so that is what we key on. A renderer that switches source closes its HTTP connection. We
    /// allow a grace period first, because a brief drop is also how a renderer *reconnects*
    /// (which the header-replay contract explicitly supports), and only treat sustained silence as
    /// the session being gone.
    async fn run_stream_watchdog(&self, epoch: u64, id: SinkId, routes: StreamRoutes) {
        // Give the receiver time to make its first fetch after LOAD.
        tokio::time::sleep(CONSUMER_GRACE).await;
        let mut absent = Duration::ZERO;
        loop {
            // Stop watching once this session is no longer the active one.
            if !self.is_current(epoch).await {
                return;
            }

            // Once the track has been fed in full the renderer has all of it and stops pulling
            // while it plays out its buffer (15s and more on some speakers); that is the track
            // ending, not a takeover, and the renderer's own report will say so.
            if routes.consumers() == 0 && !routes.is_drained() {
                absent += CONSUMER_POLL;
                if absent >= CONSUMER_GRACE {
                    tracing::warn!(
                        sink = ?id,
                        "renderer stopped consuming our stream ({}s); assuming the session was \
                         taken over and failing back to local",
                        absent.as_secs()
                    );
                    self.fail_back_to_local(epoch, &id).await;
                    return;
                }
            } else {
                absent = Duration::ZERO; // reconnected (or never really gone)
            }
            tokio::time::sleep(CONSUMER_POLL).await;
        }
    }

    /// Drop a dead network session and resume on local output from where it left off.
    async fn fail_back_to_local(&self, epoch: u64, id: &SinkId) {
        let resume_at = std::time::Duration::from_millis(self.player.snapshot().position_ms);
        let dropped = {
            let mut inner = self.inner.lock().await;
            // Only tear down if this is still the active session; a newer selection supersedes us.
            match inner.network.as_ref() {
                Some(session) if session.epoch == epoch => inner.network.take().is_some(),
                _ => false,
            }
        };
        if !dropped {
            return; // superseded by a newer sink; nothing to fail back from
        }
        // Tell the state machine the sink died (it moves `sink` back to local), then resume.
        self.player
            .engine(EngineEvent::SinkFailed(id.clone()))
            .await;
        self.restart_current(resume_at).await;
    }

    /// Handle an engine event from the playback of `generation`, ignoring stale ones.
    async fn on_engine_event(&self, generation: u64, event: EngineEvent) {
        // Decide under the lock; act after releasing it (play_index re-locks).
        let advance_to = {
            let inner = self.inner.lock().await;
            if generation != inner.generation {
                return; // stale playback; ignore entirely
            }
            match event {
                EngineEvent::Ended if inner.index + 1 < inner.queue.len() => Some(inner.index + 1),
                _ => None,
            }
        };

        match (event, advance_to) {
            (EngineEvent::Ended, Some(next)) => {
                // Spawn rather than await: this method is itself run from the engine-event
                // task that play_index spawns, so awaiting it here would be recursive.
                let controller = self.arc();
                tokio::spawn(async move {
                    let _ = controller.play_index(next).await;
                });
            }
            (EngineEvent::Ended, None) => {
                self.inner.lock().await.active = false;
                self.player.engine(EngineEvent::Ended).await;
            }
            (other, _) => self.player.engine(other).await,
        }
    }

    fn with_audio(&self, inner: &Inner, f: impl FnOnce(&AudioPlayer)) {
        if let Some(audio) = inner.audio.as_ref() {
            f(audio);
        }
    }
}

#[async_trait]
impl ControlPlane for PlaybackController {
    async fn dispatch(&self, command: Command) -> Result<()> {
        match command {
            Command::Load(track) => {
                {
                    let mut inner = self.inner.lock().await;
                    inner.queue = vec![track];
                    inner.index = 0;
                }
                self.play_index(0).await
            }
            Command::Enqueue(track) => {
                let start_at = {
                    let mut inner = self.inner.lock().await;
                    inner.queue.push(track);
                    // Auto-start if nothing is playing.
                    (!inner.active).then(|| inner.queue.len() - 1)
                };
                match start_at {
                    Some(index) => self.play_index(index).await,
                    None => {
                        // Queue grew but player state didn't change; refresh the view.
                        self.dirty.notify_one();
                        Ok(())
                    }
                }
            }
            Command::Next => {
                let next = {
                    let inner = self.inner.lock().await;
                    (inner.active && inner.index + 1 < inner.queue.len()).then_some(inner.index + 1)
                };
                match next {
                    Some(index) => self.play_index(index).await,
                    None => Err(Error::NotFound("no next track".into())),
                }
            }
            Command::Previous => {
                let prev = {
                    let inner = self.inner.lock().await;
                    (inner.active && inner.index > 0).then_some(inner.index - 1)
                };
                match prev {
                    Some(index) => self.play_index(index).await,
                    None => Err(Error::NotFound("no previous track".into())),
                }
            }
            Command::Clear => {
                {
                    let mut inner = self.inner.lock().await;
                    halt(&mut inner);
                    inner.queue.clear();
                    inner.index = 0;
                }
                self.player.command(Command::Stop).await;
                Ok(())
            }
            // Transport goes to whatever is producing sound. On a renderer that is the device
            // itself — pausing only our feed just lets it play out its buffer — and the feed
            // pauses too, or the stream would run on into a device that has stopped consuming it.
            // Its effect is observed, not assumed: the device reports `Paused`, and the player
            // follows.
            Command::Play => {
                {
                    let inner = self.inner.lock().await;
                    self.with_audio(&inner, AudioPlayer::resume);
                    with_loaded_renderer(&inner, |sink| sink.play())?;
                }
                self.player.command(Command::Play).await;
                Ok(())
            }
            Command::Pause => {
                {
                    let inner = self.inner.lock().await;
                    self.with_audio(&inner, AudioPlayer::pause);
                    with_loaded_renderer(&inner, |sink| sink.pause())?;
                }
                self.player.command(Command::Pause).await;
                Ok(())
            }
            Command::Stop => {
                halt(&mut *self.inner.lock().await);
                self.player.command(Command::Stop).await;
                Ok(())
            }
            // A renderer owns its volume; the PCM we stream it is always full scale.
            Command::SetVolume(volume) => {
                {
                    let inner = self.inner.lock().await;
                    match inner.network.as_ref() {
                        Some(session) => session.sink.set_volume(volume)?,
                        None => self.with_audio(&inner, |audio| audio.set_volume(volume)),
                    }
                }
                self.player.command(Command::SetVolume(volume)).await;
                Ok(())
            }
            Command::SetMuted(muted) => {
                {
                    let inner = self.inner.lock().await;
                    match inner.network.as_ref() {
                        Some(session) => session.sink.set_muted(muted)?,
                        None => self.with_audio(&inner, |audio| audio.set_muted(muted)),
                    }
                }
                self.player.command(Command::SetMuted(muted)).await;
                Ok(())
            }
            Command::SelectSink(id) => self.select_sink(&id).await,
            Command::Seek(position) => self.seek(position).await,
        }
    }

    fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        self.snapshots.subscribe()
    }

    fn snapshot(&self) -> PlayerSnapshot {
        self.snapshots.borrow().clone()
    }

    /// Local output plus every discovered output — one per physical device, whatever protocols
    /// reach it — local first.
    fn sinks(&self) -> Vec<SinkInfo> {
        let mut sinks = vec![SinkInfo::local()];
        if let Some(discovery) = self.discovery.as_ref() {
            sinks.extend(canon_sink::outputs(&discovery.devices().borrow()));
        }
        sinks
    }
}

/// How many times to try opening a renderer session, and how long to wait between tries.
const CONNECT_ATTEMPTS: u32 = 3;
const CONNECT_RETRY: Duration = Duration::from_secs(1);

/// Open a renderer session, allowing the speaker a moment to settle. A speaker that has just been
/// released by another protocol, or woken from standby, commonly refuses the first attempt (the
/// LS50 Wireless II answers a Cast launch with CANCELLED while it is still leaving DLNA).
async fn connect_settled(
    device: &canon_sink::DiscoveredDevice,
) -> Result<(Box<dyn Sink>, RendererEvents)> {
    let mut attempt = 1;
    loop {
        match canon_sink::connect(device).await {
            Ok(connected) => return Ok(connected),
            Err(e) if attempt < CONNECT_ATTEMPTS => {
                tracing::debug!(device = %device.name, attempt, "renderer connect failed, retrying: {e}");
                attempt += 1;
                tokio::time::sleep(CONNECT_RETRY).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Stop producing sound and invalidate any in-flight start, leaving the queue and the selected
/// output as they are.
fn halt(inner: &mut Inner) {
    if let Some(previous) = inner.audio.take() {
        previous.stop();
    }
    inner.active = false;
    inner.generation += 1;
    // Stop the renderer too — it would otherwise play out its buffer — and forget the load, so
    // the "cancelled" it reports next is not read as the track ending.
    if let Some(session) = inner.network.as_mut()
        && session.current.take().is_some()
    {
        let _ = session.sink.stop();
    }
}

/// Run a transport command on the active renderer, if it has our media loaded. With nothing
/// loaded there is nothing to play or pause, and a device asked to anyway reports an error that
/// would read as the session failing.
fn with_loaded_renderer(inner: &Inner, f: impl FnOnce(&dyn Sink) -> Result<()>) -> Result<()> {
    match inner.network.as_ref() {
        Some(session) if session.current.is_some() => f(session.sink.as_ref()),
        _ => Ok(()),
    }
}

/// Which playback generation a renderer's report about `load` belongs to — `None` if it is about
/// a stream we have moved on from.
///
/// A renderer keeps reporting on the media it has until it accepts the next load, so a report can
/// arrive after the controller has already moved on: the user skipped, sought, or stopped. The
/// load must be the one the session is playing, *and* that load must still be the current
/// generation's — between a skip and its new load being issued, the old load is still the
/// session's, but no longer what we are playing.
fn attribute(current: Option<(LoadId, u64)>, live_generation: u64, load: LoadId) -> Option<u64> {
    let (current_load, generation) = current?;
    (load == current_load && generation == live_generation).then_some(generation)
}

/// The LAN address a renderer can reach the stream server on, chosen with discovery's own
/// interface filter so a tunnel/VPN address (unreachable from the speaker) can't be picked.
fn lan_ip() -> Result<IpAddr> {
    let interfaces = canon_sink::host_interfaces();
    canon_sink::usable_interfaces(&interfaces)
        .into_iter()
        .map(|iface| iface.ip)
        .find(IpAddr::is_ipv4)
        .ok_or_else(|| Error::Sink("no usable LAN interface for the stream server".into()))
}

/// The first Tidal binding on a track, if any.
fn tidal_id(track: &TrackRef) -> Option<String> {
    track.sources.iter().find_map(|source| match source {
        SourceRef::Tidal { id } => Some(id.clone()),
        _ => None,
    })
}

/// A Symphonia probe hint for a codec.
fn codec_hint(codec: Codec) -> Option<&'static str> {
    match codec {
        Codec::Flac => Some("flac"),
        Codec::Aac | Codec::Alac => Some("m4a"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_about_the_live_load_counts() {
        assert_eq!(attribute(Some((LoadId(3), 7)), 7, LoadId(3)), Some(7));
    }

    /// canon-587a: the user skips (generation 7 -> 8) and, before the next track's load is even
    /// issued, the old track's "finished" arrives. Attributed at receipt it advanced the queue a
    /// second time; attributed to its load it is recognised as old news.
    #[test]
    fn a_finish_arriving_after_a_skip_is_stale() {
        assert_eq!(attribute(Some((LoadId(3), 7)), 8, LoadId(3)), None);
    }

    /// Once the next load is issued, the receiver goes on reporting the old media until it
    /// accepts it; those reports carry the old load.
    #[test]
    fn a_report_about_a_superseded_load_is_stale() {
        assert_eq!(attribute(Some((LoadId(4), 8)), 8, LoadId(3)), None);
    }

    /// After a stop nothing is loaded, so the "cancelled" the device reports for the stopped
    /// stream is not the track ending.
    #[test]
    fn nothing_counts_with_nothing_loaded() {
        assert_eq!(attribute(None, 8, LoadId(3)), None);
    }
}
