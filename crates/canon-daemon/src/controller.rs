//! The playback controller (yaks canon-a7d6, canon-fdf3): the daemon-level shell that turns the
//! player's decisions into actual sound.
//!
//! The player actor owns every decision — the queue, what plays next, when the queue advances,
//! where a restart resumes — and emits each one as an [`Effect`]. This controller executes them:
//! it resolves streams from the source, runs the audio engine, and drives the network output. It
//! reports what really happened back as [`EngineEvent`]s tagged with the generation of the effect
//! they belong to, and the actor, which knows which playback is current, discards stale ones.
//!
//! What stays here is what is genuinely effectful: opening and tearing down network sessions
//! (output selection has to connect before the player can play there), the per-session stream
//! server, attribution of renderer reports to the loads they describe, and the liveness watchdog.
//!
//! It implements [`ControlPlane`], so `canon-api` drives it exactly like the bare player.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use canon_audio::{AudioPlayer, Output};
use canon_core::{
    Command, ControlPlane, Effect, EngineEvent, Error, LoadId, OutputMode, PcmSink, PlayerHandle,
    PlayerSnapshot, Quality, QueueSnapshot, RendererEvent, RendererReport, Result, SettingsStore,
    Sink, SinkId, SinkInfo, Sources, TrackRef,
};
use canon_sink::{DiscoveryService, FlacTap, RendererEvents, StreamRoutes};
use tokio::sync::{Mutex, mpsc, watch};

/// What is producing sound right now.
#[derive(Default)]
struct Inner {
    /// The generation of the newest start or halt. A start whose stream resolves after a newer
    /// effect arrived is discarded instead of installed.
    latest: u64,
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
    /// The output this session drives, as settings name it: one id per speaker, whichever
    /// protocol reaches it.
    output: SinkId,
    /// Whether the current stream joins following tracks on to itself (flow mode, as it was set
    /// when the stream started).
    joins: bool,
    /// The load the renderer is playing for us, and the playback generation it was issued for.
    /// `None` before the first load and after a halt: then nothing the renderer says about its
    /// media is about ours.
    current: Option<(LoadId, u64)>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for NetworkSession {
    fn drop(&mut self) {
        self.server.abort();
    }
}

pub struct PlaybackController {
    player: PlayerHandle,
    /// Where tracks come from: one source per service, chosen among a track's bindings by
    /// policy.
    sources: Sources,
    quality: Quality,
    settings: Arc<dyn SettingsStore>,
    inner: Mutex<Inner>,
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
    /// Build the controller and start executing the player's effects.
    pub fn new(
        player: PlayerHandle,
        mut effects: mpsc::UnboundedReceiver<Effect>,
        sources: Sources,
        quality: Quality,
        settings: Arc<dyn SettingsStore>,
        discovery: Option<Arc<DiscoveryService>>,
    ) -> Arc<Self> {
        let controller = Arc::new_cyclic(|me| Self {
            player,
            sources,
            quality,
            settings,
            inner: Mutex::new(Inner::default()),
            discovery,
            sessions: AtomicU64::new(0),
            me: me.clone(),
        });
        // One executor, in order: the effects are the actor's decisions, and their order is part
        // of what was decided (halt, then start; start, then pause).
        let executor = Arc::clone(&controller);
        tokio::spawn(async move {
            while let Some(effect) = effects.recv().await {
                executor.execute(effect).await;
            }
        });
        controller
    }

    fn arc(&self) -> Arc<Self> {
        self.me.upgrade().expect("controller alive")
    }

    /// Carry out one decision of the player's.
    async fn execute(&self, effect: Effect) {
        match effect {
            Effect::Start {
                generation,
                track,
                position,
            } => {
                {
                    // Whatever was playing stops now, not once the new stream has resolved.
                    let mut inner = self.inner.lock().await;
                    inner.latest = generation;
                    if let Some(previous) = inner.audio.take() {
                        previous.stop();
                    }
                }
                // Resolving takes a network round trip or two; the executor must stay free to
                // take the next decision (a halt, another skip) meanwhile.
                let controller = self.arc();
                tokio::spawn(async move { controller.start(generation, track, position).await });
            }
            Effect::Halt { generation } => {
                let mut inner = self.inner.lock().await;
                inner.latest = generation;
                if let Some(previous) = inner.audio.take() {
                    previous.stop();
                }
                // Stop the renderer too — it would otherwise play out its buffer — and forget the
                // load, so the "cancelled" it reports next is not about anything of ours.
                if let Some(session) = inner.network.as_mut()
                    && session.current.take().is_some()
                {
                    let _ = session.sink.stop();
                }
            }
            Effect::Prepare {
                generation,
                next_generation,
                track,
            } => {
                let controller = self.arc();
                tokio::spawn(async move {
                    controller.prepare(generation, next_generation, track).await;
                });
            }
            Effect::Unprepare { next_generation } => {
                if let Some(audio) = &self.inner.lock().await.audio {
                    audio.cancel_next(next_generation);
                }
            }
            // Transport goes to whatever is producing sound. On a renderer that is the device
            // itself — pausing only our feed just lets it play out its buffer — and the feed
            // pauses too, or the stream would run on into a device that has stopped consuming it.
            // Its effect is observed, not assumed: the device reports `Paused`.
            Effect::Pause => {
                let inner = self.inner.lock().await;
                if let Some(audio) = &inner.audio {
                    audio.pause();
                }
                report_refusal(with_loaded_renderer(&inner, |sink| sink.pause()));
            }
            Effect::Resume => {
                let inner = self.inner.lock().await;
                if let Some(audio) = &inner.audio {
                    audio.resume();
                }
                report_refusal(with_loaded_renderer(&inner, |sink| sink.play()));
            }
            // A renderer owns its volume; the PCM we stream it is always full scale.
            Effect::SetVolume(volume) => {
                let inner = self.inner.lock().await;
                match (&inner.network, &inner.audio) {
                    (Some(session), _) => report_refusal(session.sink.set_volume(volume)),
                    (None, Some(audio)) => audio.set_volume(volume),
                    (None, None) => {}
                }
            }
            Effect::SetMuted(muted) => {
                let inner = self.inner.lock().await;
                match (&inner.network, &inner.audio) {
                    (Some(session), _) => report_refusal(session.sink.set_muted(muted)),
                    (None, Some(audio)) => audio.set_muted(muted),
                    (None, None) => {}
                }
            }
        }
    }

    /// Resolve `track` and start it at `position` on the selected output, unless a newer decision
    /// has superseded this one by the time the stream is ready.
    async fn start(&self, generation: u64, track: TrackRef, position: Duration) {
        // Display metadata (title/artist/duration), so the snapshot carries a real duration
        // while the stream is still resolving. Written back into the queue, so fetched once.
        let mut meta = track.meta.clone();
        if meta.duration_ms.is_none()
            && let Ok(described) = self.sources.track_meta(&track).await
        {
            meta = described.clone();
            self.player
                .engine(generation, EngineEvent::Described(described))
                .await;
        }

        let resolved = match self.sources.open(&track, self.quality, position).await {
            Ok(resolved) => resolved,
            Err(e) => {
                self.player
                    .engine(generation, EngineEvent::Failed(e.to_string()))
                    .await;
                return;
            }
        };

        let mut inner = self.inner.lock().await;
        if inner.latest != generation {
            return; // superseded while resolving: a newer start or a halt owns the output now
        }

        // Route to the selected output. A live network session gets a fresh stream for this track,
        // fed by a fresh FLAC tap (a new track means a new STREAMINFO header), and the renderer is
        // told to load it; otherwise we play locally.
        let output = match inner.network.as_mut() {
            Some(session) => {
                session.joins =
                    self.settings.get().output(&session.output).mode == OutputMode::Flow;
                let (path, broadcaster) = session.routes.open();
                let tap = match FlacTap::new(
                    broadcaster,
                    resolved.info.sample_rate,
                    resolved.info.channels,
                    FLAC_BITS,
                ) {
                    Ok(tap) => tap,
                    Err(e) => {
                        drop(inner);
                        self.player
                            .engine(generation, EngineEvent::Failed(e.to_string()))
                            .await;
                        return;
                    }
                };
                let url = format!("{}{path}", session.base_url);
                match session.sink.load(&url, &meta) {
                    Ok(load) => session.current = Some((load, generation)),
                    Err(e) => {
                        drop(inner);
                        self.player
                            .engine(generation, EngineEvent::Failed(e.to_string()))
                            .await;
                        return;
                    }
                }
                Output::Network {
                    sink: Box::new(tap) as Box<dyn PcmSink>,
                    joins: session.joins,
                }
            }
            None => Output::Local,
        };
        let local = matches!(output, Output::Local);

        // The engine's events belong to this run — every track joined on to it included: tag
        // them, and let the actor judge.
        let (events_tx, mut events_rx) = mpsc::unbounded_channel::<EngineEvent>();
        let player = self.player.clone();
        tokio::spawn(async move {
            while let Some(event) = events_rx.recv().await {
                player.engine(generation, event).await;
            }
        });

        let audio = AudioPlayer::start(
            resolved.input,
            resolved.info.codec.extension_hint().map(str::to_owned),
            self.player.clock(),
            events_tx,
            resolved.start_ms,
            output,
        );
        // A fresh engine starts at full gain; the local output must come up at the level the
        // player says it is at. (A renderer keeps its own volume across loads.)
        if local {
            let snapshot = self.player.snapshot();
            audio.set_volume(snapshot.volume);
            audio.set_muted(snapshot.muted);
        }
        inner.audio = Some(audio);
    }

    /// Resolve `track` and hand it to the playback of `generation` as its successor, for a
    /// gapless join. Best effort: if anything here fails, or the playback has moved on, the
    /// current track simply ends and the player starts the next one as usual.
    async fn prepare(&self, generation: u64, next_generation: u64, track: TrackRef) {
        if track.meta.duration_ms.is_none()
            && let Ok(described) = self.sources.track_meta(&track).await
        {
            self.player
                .engine(next_generation, EngineEvent::Described(described))
                .await;
        }
        let resolved = match self
            .sources
            .open(&track, self.quality, Duration::ZERO)
            .await
        {
            Ok(resolved) => resolved,
            Err(e) => {
                tracing::debug!("next track not prepared: {e}");
                return;
            }
        };
        let inner = self.inner.lock().await;
        // Only the run it was prepared for can take it, and only on a stream that joins.
        let joins = inner.network.as_ref().is_none_or(|session| session.joins);
        if inner.latest != generation || !joins {
            return;
        }
        if let Some(audio) = &inner.audio {
            audio.prepare_next(
                resolved.input,
                resolved.info.codec.extension_hint().map(str::to_owned),
                next_generation,
            );
        }
    }

    /// Switch the active output. The player restarts the current track there, at the position
    /// the listener had reached.
    ///
    /// Selecting `local` tears down any network session; selecting a discovered renderer connects to
    /// it and stands up a LAN stream server bound to the interface discovery chose. One output is
    /// active at a time, so this is a restart rather than a re-route of a live stream.
    async fn select_sink(&self, id: &SinkId) -> Result<()> {
        if SinkInfo::is_local(id) {
            self.inner.lock().await.network.take(); // Drop ends the renderer session.
            return self.player.command(Command::SelectSink(id.clone())).await;
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
                    let _ = self
                        .player
                        .command(Command::SelectSink(SinkInfo::local().id))
                        .await;
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
        self.player.command(Command::SelectSink(id.clone())).await
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

        // Settings name the speaker, not the protocol endpoint we happen to reach it by.
        let output = self
            .discovery
            .as_ref()
            .and_then(|discovery| {
                canon_sink::outputs(&discovery.devices().borrow())
                    .into_iter()
                    .find(|output| output.protocols.iter().any(|p| p.id == device.id))
                    .map(|output| output.id)
            })
            .unwrap_or_else(|| device.id.clone());

        Ok(NetworkSession {
            sink,
            epoch,
            routes,
            base_url,
            host: device.addr.ip(),
            output,
            joins: false,
            current: None,
            server,
        })
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
                attribute(session.current, load)
            };
            // Device reports enter as engine events, never as commands: a command is user intent,
            // and the state machine must be able to tell "the speaker is playing" from "someone
            // pressed play". Each describes one load, so it goes to the playback that load was for
            // — and the player drops it if that playback is no longer current.
            let forward = match event {
                RendererEvent::State(state) => Some(EngineEvent::RendererState(state)),
                RendererEvent::Position(position) => Some(EngineEvent::RendererPosition(position)),
                // The same end-of-track as the local engine's, so the queue advances identically
                // on either output.
                RendererEvent::Ended => Some(EngineEvent::Ended),
                // Losing the renderer is about the session, not one stream: honoured whatever load
                // is current (a takeover mid-track-change must not be dropped).
                RendererEvent::Superseded(why) | RendererEvent::Failed(why) => {
                    tracing::warn!(sink = ?id, "renderer session lost: {why}");
                    self.fail_back_to_local(epoch, &id).await;
                    return;
                }
            };
            if let (Some(generation), Some(event)) = (generation, forward) {
                self.player.engine(generation, event).await;
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
            let Some(loaded) = self.loaded(epoch).await else {
                return;
            };

            // A renderer with nothing of ours loaded (an empty queue, after stop, after a track
            // that failed to open) has nothing to pull. Once the track has been fed in full the
            // renderer has all of it and stops pulling while it plays out its buffer (15s and
            // more on some speakers); that is the track ending, not a takeover, and the
            // renderer's own report will say so.
            if loaded && routes.consumers() == 0 && !routes.is_drained() {
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

    /// Whether the network session `epoch` has one of our loads on its renderer, or `None` if it
    /// is no longer the live session.
    async fn loaded(&self, epoch: u64) -> Option<bool> {
        self.inner
            .lock()
            .await
            .network
            .as_ref()
            .filter(|session| session.epoch == epoch)
            .map(|session| session.current.is_some())
    }

    /// Drop a dead network session, and tell the player, which resumes on local output from where
    /// the listener was.
    async fn fail_back_to_local(&self, epoch: u64, id: &SinkId) {
        let dropped = {
            let mut inner = self.inner.lock().await;
            // Only tear down if this is still the active session; a newer selection supersedes us.
            match inner.network.as_ref() {
                Some(session) if session.epoch == epoch => inner.network.take().is_some(),
                _ => false,
            }
        };
        if dropped {
            self.player.sink_failed(id.clone()).await;
        }
    }
}

#[async_trait]
impl ControlPlane for PlaybackController {
    /// Output selection has to connect before the player can play there; everything else is the
    /// player's to decide.
    async fn dispatch(&self, command: Command) -> Result<()> {
        match command {
            Command::SelectSink(id) => self.select_sink(&id).await,
            other => self.player.command(other).await,
        }
    }

    fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        self.player.subscribe()
    }

    fn snapshot(&self) -> PlayerSnapshot {
        self.player.snapshot()
    }

    fn queue(&self) -> QueueSnapshot {
        self.player.queue()
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

/// Run a transport command on the active renderer, if it has our media loaded. With nothing
/// loaded there is nothing to play or pause, and a device asked to anyway reports an error that
/// would read as the session failing.
fn with_loaded_renderer(inner: &Inner, f: impl FnOnce(&dyn Sink) -> Result<()>) -> Result<()> {
    match inner.network.as_ref() {
        Some(session) if session.current.is_some() => f(session.sink.as_ref()),
        _ => Ok(()),
    }
}

/// A command the renderer's session could not even queue means the session is gone; its event
/// stream closing is what fails back, so here it is only worth a line in the log.
fn report_refusal(outcome: Result<()>) {
    if let Err(e) = outcome {
        tracing::warn!("renderer command not delivered: {e}");
    }
}

/// Which playback generation a renderer's report about `load` belongs to — `None` if it is about
/// media we are no longer playing there.
///
/// A renderer keeps reporting on the media it has until it accepts the next load, and after a halt
/// it reports the media we stopped. Only a report about the session's current load is about a
/// playback of ours; whether that playback is still the *current* one is the player's call.
fn attribute(current: Option<(LoadId, u64)>, load: LoadId) -> Option<u64> {
    let (current_load, generation) = current?;
    (load == current_load).then_some(generation)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_about_the_current_load_goes_to_its_playback() {
        assert_eq!(attribute(Some((LoadId(3), 7)), LoadId(3)), Some(7));
    }

    /// Once the next load is issued, the receiver goes on reporting the old media until it
    /// accepts it; those reports carry the old load.
    #[test]
    fn a_report_about_a_superseded_load_goes_nowhere() {
        assert_eq!(attribute(Some((LoadId(4), 8)), LoadId(3)), None);
    }

    /// After a halt nothing is loaded, so the "cancelled" the device reports for the stopped
    /// stream is not the track ending.
    #[test]
    fn nothing_counts_with_nothing_loaded() {
        assert_eq!(attribute(None, LoadId(3)), None);
    }
}
