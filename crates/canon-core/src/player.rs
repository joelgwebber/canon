//! The player state actor: the single owner of playback state.
//!
//! This is the runtime realisation of the state core. One task owns the transport state, the
//! play queue, and the emit loop; the WebSocket+JSON API and the MCP layer talk to it only
//! through [`Command`]s, and the audio/sink layers feed [`EngineEvent`]s back in. Because
//! *every* input that can change playback reality — including device loss, sink failure, and a
//! track ending — is a message that flows through this one actor and re-emits, the published
//! [`PlayerSnapshot`] can never silently desync from what is actually happening (the class of
//! bug behind tideway tide-2f85).
//!
//! ## Decisions in here, effects out there
//!
//! The actor *decides*: which track plays next, when the queue advances, where a restart
//! resumes. It does no I/O. Each decision that needs the world to act goes out as an
//! [`Effect`] — start this track here, halt, pause — and the daemon's playback controller
//! carries it out and reports what actually happened as engine events. Every playback start
//! gets a new **generation**; effects carry it, and engine events come back tagged with the
//! generation they belong to, so a report about a playback we have already moved on from (a
//! track skipped, sought, or stopped) is recognised as stale here, in one place, by the one
//! thing that knows which playback is current.
//!
//! Position is not stored: it is derived from the shared [`FrameClock`] the realtime output
//! callback advances, or from the renderer's own reports, so a stalled output shows up as a
//! position that stops advancing rather than an invisibly frozen emitter.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    Command, Error, FrameClock, PlaybackState, PlayerSnapshot, PositionDrive, QueueView, Reconcile,
    RendererClock, RendererState, Result, SinkId, SinkInfo, TrackMeta, TrackRef,
};

/// How often the actor refreshes derived position while playing. Snapshots also carry
/// a `rate`, so clients interpolate between these and this can stay coarse.
const POSITION_TICK: Duration = Duration::from_millis(250);

/// How far ahead of the end of a track its successor is made ready for a gapless hand-off.
/// Resolving a stream is a network round trip or two and the decoder must have probed it before
/// the current track's last samples leave the ring; this leaves room for a slow source.
const PRELOAD_LEAD: Duration = Duration::from_secs(15);

/// How long, after a pause or play on a renderer, a report contradicting it is taken to predate
/// it. A few polls (renderers are polled twice a second): long enough to cover a command still in
/// flight, short enough that a device which really ignored the command is believed promptly.
const CONFIRM_WINDOW: Duration = Duration::from_secs(3);

/// The condition a renderer should report once it has carried out our last transport command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Paused,
    Playing,
}

impl Expect {
    /// Whether `reported` is the device doing what we asked. A renderer resuming commonly passes
    /// through buffering, which is it obeying, not contradicting.
    fn confirmed_by(self, reported: RendererState) -> bool {
        match self {
            Expect::Paused => reported == RendererState::Paused,
            Expect::Playing => {
                matches!(reported, RendererState::Playing | RendererState::Buffering)
            }
        }
    }
}

/// Signals the audio/sink layers feed back into the state machine — the "reality
/// changed" inputs. Making these explicit transitions (rather than out-of-band
/// mutations on a side thread) is exactly what keeps the emitted state truthful.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// The decoder opened the stream and playback begins. Carries the source sample
    /// rate (to rebase the frame clock), the known duration if any, the timeline
    /// position this stream starts at (non-zero after a seek), and where position will
    /// come from for this stream, so the player rebases and positions in one step.
    Loaded {
        sample_rate: u32,
        duration_ms: Option<u64>,
        start_ms: u64,
        drive: PositionDrive,
    },
    /// The source described the track being started: its display metadata (title, artists,
    /// duration). Written back into the queue entry, so it is fetched once per entry.
    Described(TrackMeta),
    /// A network renderer reported what it is doing. This is a *report*, not a
    /// confirmation of a command we sent, which is why it is an engine event and not a
    /// [`Command`]: commands are user intent, and laundering device status through them
    /// makes the two indistinguishable to the state machine.
    RendererState(RendererState),
    /// A network renderer reported its position on the stream it was handed. Folded into the
    /// renderer clock as a correction, not as a seek.
    RendererPosition(Duration),
    /// The playback reached its end: the decoder on the local path, the renderer on a network
    /// one. The queue advances on this, identically for both.
    Ended,
    /// Unrecoverable playback error.
    Failed(String),
    /// The listener has crossed from the current track into the prepared one, without a break:
    /// the engine joined them in one output (a gapless hand-off). `to` is the prepared playback's
    /// generation; the engine has already rebased the clock onto the new track.
    Advanced { to: u64 },
    /// The output device/stream was reopened for the *same* track (device loss +
    /// recovery, sleep/wake). Position continues; only the output identity changed.
    DeviceChanged,
    /// The active network sink died; the player fails back to local output and resumes there.
    /// About the output, not one playback, so it is never stale.
    SinkFailed(SinkId),
}

/// What the actor needs the world to do. Each is a decision already made; the executor (the
/// daemon's playback controller) carries it out, and reports what really happened as
/// [`EngineEvent`]s tagged with the effect's generation.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Produce `track` from `position` on the selected output. Supersedes every earlier
    /// playback: whatever was playing stops.
    Start {
        generation: u64,
        track: TrackRef,
        position: Duration,
    },
    /// Stop producing sound, on every output.
    Halt {
        generation: u64,
    },
    /// Get `track` ready to follow the playback of `generation` without a break: it becomes the
    /// playback `next_generation` when the listener reaches it (reported as
    /// [`EngineEvent::Advanced`]). If the hand-off can't happen, the current playback simply ends
    /// and the next one starts as usual.
    Prepare {
        generation: u64,
        next_generation: u64,
        track: TrackRef,
    },
    /// Hold the current playback where it is.
    Pause,
    /// Continue the current playback.
    Resume,
    SetVolume(f32),
    SetMuted(bool),
}

/// The queue's contents, published alongside the snapshot. Separate because the snapshot goes
/// out several times a second and a long queue need not; `revision` matches the snapshot's
/// [`QueueView::revision`], so a client refetches only when it moved.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueSnapshot {
    pub revision: u64,
    pub index: usize,
    pub tracks: Arc<Vec<TrackRef>>,
}

impl QueueSnapshot {
    fn empty() -> Self {
        Self {
            revision: 0,
            index: 0,
            tracks: Arc::new(Vec::new()),
        }
    }
}

enum Input {
    Command(Command, Option<oneshot::Sender<Result<()>>>),
    Engine(Option<u64>, EngineEvent),
}

/// Whether an input actually changed anything. Only a transition bumps the snapshot `seq`,
/// which is what clients read as "something happened".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transition {
    Yes,
    No,
}

/// A cheap, cloneable handle to the running player actor.
#[derive(Clone)]
pub struct PlayerHandle {
    input: mpsc::Sender<Input>,
    snapshots: watch::Receiver<PlayerSnapshot>,
    queue: watch::Receiver<QueueSnapshot>,
    clock: Arc<FrameClock>,
}

impl PlayerHandle {
    /// Spawn a state-only actor: its effects go nowhere. For tests, and for any headless
    /// "state mirror" use.
    #[must_use]
    pub fn spawn() -> PlayerHandle {
        Self::spawn_inner(None)
    }

    /// Spawn the actor with an executor: every [`Effect`] it decides on arrives on the returned
    /// receiver, in order.
    #[must_use]
    pub fn spawn_with_effects() -> (PlayerHandle, mpsc::UnboundedReceiver<Effect>) {
        let (effects_tx, effects_rx) = mpsc::unbounded_channel();
        (Self::spawn_inner(Some(effects_tx)), effects_rx)
    }

    fn spawn_inner(effects: Option<mpsc::UnboundedSender<Effect>>) -> PlayerHandle {
        let clock = Arc::new(FrameClock::new());
        let (input_tx, input_rx) = mpsc::channel(64);
        let (snap_tx, snap_rx) = watch::channel(PlayerSnapshot::idle());
        let (queue_tx, queue_rx) = watch::channel(QueueSnapshot::empty());
        let actor = Actor::new(clock.clone(), snap_tx, queue_tx, effects);
        tokio::spawn(actor.run(input_rx));
        PlayerHandle {
            input: input_tx,
            snapshots: snap_rx,
            queue: queue_rx,
            clock,
        }
    }

    /// Send a control-plane command (from the API / MCP layer) and wait for the actor's verdict:
    /// `Err` when it cannot apply (no next track, nothing to seek).
    ///
    /// # Errors
    /// The actor's rejection of the command, or [`Error::Unsupported`] if the actor is gone.
    pub async fn command(&self, cmd: Command) -> Result<()> {
        let (reply, verdict) = oneshot::channel();
        if self
            .input
            .send(Input::Command(cmd, Some(reply)))
            .await
            .is_err()
        {
            return Err(Error::Unsupported("the player has stopped".into()));
        }
        verdict
            .await
            .unwrap_or_else(|_| Err(Error::Unsupported("the player has stopped".into())))
    }

    /// Feed an engine event belonging to the playback of `generation` (from an [`Effect`]).
    /// Events for a playback that is no longer current are dropped by the actor.
    pub async fn engine(&self, generation: u64, event: EngineEvent) {
        let _ = self
            .input
            .send(Input::Engine(Some(generation), event))
            .await;
    }

    /// Report that a network sink died. Not tied to any one playback.
    pub async fn sink_failed(&self, id: SinkId) {
        let _ = self
            .input
            .send(Input::Engine(None, EngineEvent::SinkFailed(id)))
            .await;
    }

    /// Feed an engine event as belonging to whatever playback is current. For tests, which have
    /// no executor to learn generations from.
    #[cfg(test)]
    pub(crate) async fn engine_now(&self, event: EngineEvent) {
        let _ = self.input.send(Input::Engine(None, event)).await;
    }

    /// Subscribe to the authoritative snapshot stream (latest value + changes). Late
    /// joiners get the current snapshot immediately, then every change.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        self.snapshots.clone()
    }

    /// The current authoritative snapshot.
    #[must_use]
    pub fn snapshot(&self) -> PlayerSnapshot {
        self.snapshots.borrow().clone()
    }

    /// The queue's current contents.
    #[must_use]
    pub fn queue(&self) -> QueueSnapshot {
        self.queue.borrow().clone()
    }

    /// The shared frame clock. Only the realtime output callback should advance it.
    #[must_use]
    pub fn clock(&self) -> Arc<FrameClock> {
        Arc::clone(&self.clock)
    }
}

struct Actor {
    seq: u64,
    state: PlaybackState,
    track: Option<TrackRef>,
    duration_ms: Option<u64>,
    volume: f32,
    muted: bool,
    sink: Option<SinkId>,
    error: Option<String>,
    queue: Vec<TrackRef>,
    /// The current entry of `queue`: the one playing, or the one that last played.
    index: usize,
    /// Bumped whenever the queue's contents or position change, so clients know to refetch it.
    queue_revision: u64,
    /// The current playback.
    generation: u64,
    /// The last generation handed out, to a playback or a prepared one. One counter for both, so
    /// no two playbacks ever share a generation — even one prepared and then abandoned.
    issued: u64,
    /// The next entry, made ready for a gapless hand-off: its generation and queue index.
    prepared: Option<(u64, usize)>,
    clock: Arc<FrameClock>,
    /// Present exactly while the active output reports its own position. When it is set it
    /// *is* the position: the frame clock on that path counts frames fed to the encoder,
    /// which run seconds ahead of what the listener hears.
    renderer: Option<RendererClock>,
    /// After a pause or play on a renderer: what it should report next, and until when a
    /// contradicting report is taken to be older than the command.
    expect: Option<(Expect, tokio::time::Instant)>,
    snap_tx: watch::Sender<PlayerSnapshot>,
    queue_tx: watch::Sender<QueueSnapshot>,
    effects: Option<mpsc::UnboundedSender<Effect>>,
    last_emitted_position_ms: u64,
}

impl Actor {
    fn new(
        clock: Arc<FrameClock>,
        snap_tx: watch::Sender<PlayerSnapshot>,
        queue_tx: watch::Sender<QueueSnapshot>,
        effects: Option<mpsc::UnboundedSender<Effect>>,
    ) -> Self {
        Self {
            seq: 0,
            state: PlaybackState::Idle,
            track: None,
            duration_ms: None,
            volume: 1.0,
            muted: false,
            sink: None,
            error: None,
            queue: Vec::new(),
            index: 0,
            queue_revision: 0,
            generation: 0,
            issued: 0,
            prepared: None,
            clock,
            renderer: None,
            expect: None,
            snap_tx,
            queue_tx,
            effects,
            last_emitted_position_ms: 0,
        }
    }

    async fn run(mut self, mut input_rx: mpsc::Receiver<Input>) {
        let mut tick = tokio::time::interval(POSITION_TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                message = input_rx.recv() => {
                    match message {
                        Some(input) => {
                            // Only a real change bumps seq. A device that reports its
                            // condition twice a second, a position correction that sharpens an
                            // estimate clients are already interpolating, and a command that
                            // changes nothing are all news to nobody — and a client reads a new
                            // seq as "something happened".
                            if self.handle(input) == Transition::Yes {
                                self.seq += 1;
                            }
                            self.emit();
                        }
                        // All handles dropped: nothing can command us again.
                        None => break,
                    }
                }
                _ = tick.tick() => {
                    self.maybe_prepare();
                    // Position-only refresh (same seq); skip when nothing moved.
                    if self.state == PlaybackState::Playing
                        && self.position_ms() != self.last_emitted_position_ms
                    {
                        self.emit();
                    }
                }
            }
        }
    }

    fn handle(&mut self, input: Input) -> Transition {
        let transition = match input {
            Input::Command(cmd, reply) => {
                let outcome = self.handle_command(cmd);
                let transition = *outcome.as_ref().unwrap_or(&Transition::No);
                if let Some(reply) = reply {
                    let _ = reply.send(outcome.map(|_| ()));
                }
                transition
            }
            // What the source said about the prepared entry, before it plays.
            Input::Engine(Some(generation), EngineEvent::Described(meta))
                if self
                    .prepared
                    .is_some_and(|(prepared, _)| prepared == generation) =>
            {
                let index = self.prepared.map_or(0, |(_, index)| index);
                if let Some(entry) = self.queue.get_mut(index) {
                    entry.meta = meta;
                    self.queue_revision += 1;
                }
                Transition::Yes
            }
            Input::Engine(Some(generation), event)
                if generation != self.generation
                    && !matches!(event, EngineEvent::SinkFailed(_)) =>
            {
                // A report about a playback we have already moved on from: a track skipped,
                // sought or stopped. Acting on it is how a finished track once advanced the
                // queue a second time (canon-587a).
                Transition::No
            }
            Input::Engine(_, event) => self.handle_engine(event),
        };
        // The renderer clock runs exactly while we are playing. Restoring that here, once,
        // means no input can leave the stopwatch disagreeing with the state machine.
        if let Some(clock) = self.renderer.as_mut() {
            if self.state == PlaybackState::Playing {
                clock.resume();
            } else {
                clock.pause();
            }
        }
        transition
    }

    /// Whether a track is in play: being started, playing, or paused. The queue advances and
    /// restarts only then.
    fn in_play(&self) -> bool {
        matches!(
            self.state,
            PlaybackState::Loading | PlaybackState::Playing | PlaybackState::Paused
        )
    }

    fn effect(&self, effect: Effect) {
        if let Some(effects) = &self.effects {
            let _ = effects.send(effect);
        }
    }

    /// Start the queue entry at `index` from `position`: a new playback, superseding the last.
    fn start(&mut self, index: usize, position: Duration) {
        self.issued += 1;
        self.generation = self.issued;
        self.prepared = None;
        self.index = index;
        self.queue_revision += 1;
        let track = self.queue[index].clone();
        self.expect = None;
        self.duration_ms = track.meta.duration_ms;
        self.track = Some(track.clone());
        self.error = None;
        self.state = PlaybackState::Loading;
        self.effect(Effect::Start {
            generation: self.generation,
            track,
            position,
        });
    }

    /// Stop producing sound, forgetting the playback (the queue is kept).
    fn halt(&mut self) {
        self.issued += 1;
        self.generation = self.issued;
        self.prepared = None;
        self.expect = None;
        self.state = PlaybackState::Idle;
        self.track = None;
        self.duration_ms = None;
        self.error = None;
        self.clock.reset(0);
        self.renderer = None;
        self.effect(Effect::Halt {
            generation: self.generation,
        });
    }

    /// Near the end of a track, get the next one ready so the output can carry straight on into
    /// it. Local output only for now: a network renderer is handed one track per stream, and
    /// joining tracks there is flow mode (canon-77f8).
    fn maybe_prepare(&mut self) {
        let next = self.index + 1;
        if self.renderer.is_some()
            || self.state != PlaybackState::Playing
            || self.prepared.is_some()
            || next >= self.queue.len()
        {
            return;
        }
        let Some(duration_ms) = self.duration_ms else {
            return;
        };
        let lead = u64::try_from(PRELOAD_LEAD.as_millis()).unwrap_or(u64::MAX);
        if self.position_ms().saturating_add(lead) < duration_ms {
            return;
        }
        self.issued += 1;
        self.prepared = Some((self.issued, next));
        self.effect(Effect::Prepare {
            generation: self.generation,
            next_generation: self.issued,
            track: self.queue[next].clone(),
        });
    }

    /// On a renderer, a command takes a poll or so to land, and the reports in between still
    /// describe the device before it. Note what it should say next (see `CONFIRM_WINDOW`).
    fn await_renderer(&mut self, expect: Expect) {
        if self.renderer.is_some() {
            self.expect = Some((expect, tokio::time::Instant::now() + CONFIRM_WINDOW));
        }
    }

    /// Restart the current entry where the listener is — after the output changed under it.
    fn restart_here(&mut self) {
        if self.in_play() {
            let position = Duration::from_millis(self.position_ms());
            self.start(self.index, position);
        }
    }

    fn reconcile(&mut self, reported: Duration) {
        let Some(clock) = self.renderer.as_mut() else {
            // No renderer clock means the local callback owns position; a stale report
            // from a sink we already switched away from must not move it.
            return;
        };
        let derived_ms = clock.position_ms();
        let reported_ms = u64::try_from(reported.as_millis()).unwrap_or(u64::MAX);
        match clock.reconcile(reported) {
            Reconcile::Snapped { drift_ms } => {
                tracing::debug!(
                    derived_ms,
                    reported_ms,
                    drift_ms,
                    "renderer position snapped"
                );
            }
            // Trace, not debug: this is the steady state at ~2/sec. Watching the drift
            // series converge toward zero is how you tell reconciliation is working.
            Reconcile::Slewed { by_ms } => {
                tracing::trace!(
                    derived_ms,
                    reported_ms,
                    by_ms,
                    "renderer position reconciled"
                );
            }
        }
    }

    fn handle_command(&mut self, cmd: Command) -> Result<Transition> {
        match cmd {
            Command::Load(track) => {
                self.queue = vec![track];
                self.start(0, Duration::ZERO);
            }
            Command::Enqueue(track) => {
                self.queue.push(track);
                self.queue_revision += 1;
                // Nothing playing: the new entry starts. Otherwise it waits its turn.
                if !self.in_play() {
                    self.start(self.queue.len() - 1, Duration::ZERO);
                }
            }
            Command::Next => {
                if self.index + 1 >= self.queue.len() {
                    return Err(Error::NotFound("no next track".into()));
                }
                self.start(self.index + 1, Duration::ZERO);
            }
            Command::Previous => {
                if self.index == 0 || self.queue.is_empty() {
                    return Err(Error::NotFound("no previous track".into()));
                }
                self.start(self.index - 1, Duration::ZERO);
            }
            Command::Clear => {
                self.queue.clear();
                self.index = 0;
                self.queue_revision += 1;
                self.halt();
            }
            Command::Play => {
                if self.state != PlaybackState::Paused {
                    return Ok(Transition::No);
                }
                self.state = PlaybackState::Playing;
                self.await_renderer(Expect::Playing);
                self.effect(Effect::Resume);
            }
            Command::Pause => {
                if self.state != PlaybackState::Playing {
                    return Ok(Transition::No);
                }
                self.state = PlaybackState::Paused;
                self.await_renderer(Expect::Paused);
                self.effect(Effect::Pause);
            }
            Command::Stop => {
                if self.state == PlaybackState::Idle {
                    return Ok(Transition::No);
                }
                self.halt();
            }
            // Seeking restarts the entry at the new position: a network renderer is handed a
            // stream that begins there, and the local path reopens its input there too.
            Command::Seek(position) => {
                if !self.in_play() {
                    return Err(Error::Unsupported("nothing to seek".into()));
                }
                self.start(self.index, position);
            }
            Command::SetVolume(volume) => {
                let volume = volume.clamp(0.0, 1.0);
                if (volume - self.volume).abs() < f32::EPSILON {
                    return Ok(Transition::No);
                }
                self.volume = volume;
                self.effect(Effect::SetVolume(volume));
            }
            Command::SetMuted(muted) => {
                if muted == self.muted {
                    return Ok(Transition::No);
                }
                self.muted = muted;
                self.effect(Effect::SetMuted(muted));
            }
            // The executor has already opened the new output; what is left is to play there.
            Command::SelectSink(id) => {
                self.sink = Some(id);
                self.restart_here();
            }
        }
        Ok(Transition::Yes)
    }

    fn handle_engine(&mut self, event: EngineEvent) -> Transition {
        match event {
            EngineEvent::Loaded {
                sample_rate,
                duration_ms,
                start_ms,
                drive,
            } => {
                self.clock.reset(sample_rate);
                if start_ms > 0 {
                    self.clock.seek(Duration::from_millis(start_ms));
                }
                if duration_ms.is_some() {
                    self.duration_ms = duration_ms;
                }
                self.error = None;
                self.renderer = match drive {
                    PositionDrive::Frames => None,
                    PositionDrive::Renderer => {
                        Some(RendererClock::new(Duration::from_millis(start_ms)))
                    }
                };
                self.state = match drive {
                    // Local output is playing the moment the stream opens: we are the ones
                    // feeding the device.
                    PositionDrive::Frames => PlaybackState::Playing,
                    // A renderer has only been handed a URL. It is still buffering, and
                    // saying "playing" here is what makes position run ahead of the audio
                    // and then jump backward when the first real report lands.
                    PositionDrive::Renderer => PlaybackState::Loading,
                };
                Transition::Yes
            }
            EngineEvent::Described(meta) => {
                if meta.duration_ms.is_some() {
                    self.duration_ms = meta.duration_ms;
                }
                if let Some(entry) = self.queue.get_mut(self.index) {
                    entry.meta = meta.clone();
                    self.queue_revision += 1;
                }
                if let Some(track) = self.track.as_mut() {
                    track.meta = meta;
                }
                Transition::Yes
            }
            // A renderer restates its condition on every poll, so this is only news when it
            // actually differs. It has to keep arriving, though: the device is the authority,
            // and a report suppressed upstream is how the player ends up believing something
            // the speaker is not doing.
            EngineEvent::RendererState(reported) => {
                // A report about a renderer stream we don't have — we stopped, or switched to
                // local — describes nothing we are doing. Acting on it is how a speaker still
                // draining its buffer once revived a stopped player to "playing" with no track.
                if self.renderer.is_none() {
                    return Transition::No;
                }
                // Just after a pause or play, a report saying otherwise was most likely taken
                // before the device got the command: without this, pausing flickered
                // paused -> playing -> paused on a real speaker. Once the device confirms, or the
                // window passes, it is the authority again — a device that really ignored the
                // command still gets its say, a few seconds later.
                if let Some((expect, until)) = self.expect {
                    if expect.confirmed_by(reported) || tokio::time::Instant::now() >= until {
                        self.expect = None;
                    } else {
                        return Transition::No;
                    }
                }
                let state = match reported {
                    RendererState::Playing => PlaybackState::Playing,
                    RendererState::Paused => PlaybackState::Paused,
                    RendererState::Buffering => PlaybackState::Loading,
                };
                if self.state == state {
                    return Transition::No;
                }
                self.state = state;
                Transition::Yes
            }
            EngineEvent::RendererPosition(reported) => {
                self.reconcile(reported);
                Transition::No
            }
            // The queue advances here, on the one input that says the track really finished —
            // whichever output it played on.
            EngineEvent::Ended => {
                if !self.in_play() {
                    return Transition::No;
                }
                if self.index + 1 < self.queue.len() {
                    self.start(self.index + 1, Duration::ZERO);
                } else {
                    self.state = PlaybackState::Ended;
                }
                Transition::Yes
            }
            EngineEvent::Failed(message) => {
                self.state = PlaybackState::Error;
                self.error = Some(message);
                Transition::Yes
            }
            // Straight on into the prepared entry: a new playback, but not a restart — nothing was
            // stopped, loaded, or buffered, and the engine has already moved the clock across.
            EngineEvent::Advanced { to } => {
                let Some((prepared, index)) = self.prepared.take() else {
                    return Transition::No;
                };
                if prepared != to {
                    return Transition::No;
                }
                self.generation = to;
                self.index = index;
                self.queue_revision += 1;
                let track = self.queue[index].clone();
                self.duration_ms = track.meta.duration_ms;
                self.track = Some(track);
                self.error = None;
                Transition::Yes
            }
            // Same track continues through a reopened output: only mark the
            // discontinuity, then re-emit so the view tracks reality.
            EngineEvent::DeviceChanged => {
                self.clock.mark_device_change();
                Transition::Yes
            }
            EngineEvent::SinkFailed(id) => {
                if self.sink.as_ref() != Some(&id) {
                    return Transition::No;
                }
                // Fail back to local, and carry on from where the listener was: playback is
                // never left wedged on an output that is gone.
                self.sink = Some(SinkInfo::local().id);
                self.restart_here();
                Transition::Yes
            }
        }
    }

    fn position_ms(&self) -> u64 {
        // A renderer clock is only ever set for a loaded stream, and it keeps a truthful
        // frozen position through buffering (`Loading`) — unlike the local path, where
        // `Loading` really does mean nothing has played yet.
        let position = if let Some(clock) = &self.renderer {
            clock.position_ms()
        } else {
            match self.state {
                PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Ended => {
                    self.clock.position_ms()
                }
                PlaybackState::Idle | PlaybackState::Loading | PlaybackState::Error => 0,
            }
        };
        // A renderer's clock is extrapolated between its reports, and the report that ends the
        // track comes a poll or two after the audio did; nothing plays past the end of a track.
        self.duration_ms
            .map_or(position, |duration| position.min(duration))
    }

    fn snapshot(&self) -> PlayerSnapshot {
        PlayerSnapshot {
            seq: self.seq,
            state: self.state,
            track: self.track.clone(),
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms,
            rate: if self.state == PlaybackState::Playing {
                1.0
            } else {
                0.0
            },
            volume: self.volume,
            muted: self.muted,
            sink: self.sink.clone(),
            error: self.error.clone(),
            queue: QueueView {
                len: self.queue.len(),
                index: self.index,
                revision: self.queue_revision,
            },
        }
    }

    fn emit(&mut self) {
        let snapshot = self.snapshot();
        self.last_emitted_position_ms = snapshot.position_ms;
        if self.queue_tx.borrow().revision != self.queue_revision {
            self.queue_tx.send_replace(QueueSnapshot {
                revision: self.queue_revision,
                index: self.index,
                tracks: Arc::new(self.queue.clone()),
            });
        }
        // send_replace is infallible even with no live receivers.
        self.snap_tx.send_replace(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityId, SourceRef, TrackMeta};

    fn track(title: &str, duration_ms: u64) -> TrackRef {
        TrackRef {
            id: EntityId::new(),
            meta: TrackMeta {
                title: title.to_string(),
                duration_ms: Some(duration_ms),
                ..Default::default()
            },
            sources: vec![SourceRef::Tidal {
                id: "1".to_string(),
            }],
        }
    }

    fn loaded(sample_rate: u32, drive: PositionDrive) -> EngineEvent {
        EngineEvent::Loaded {
            sample_rate,
            duration_ms: None,
            start_ms: 0,
            drive,
        }
    }

    /// Await the next snapshot whose `seq` advanced past `from_seq`, ignoring
    /// position-only tick refreshes (which keep the same seq).
    async fn next_transition(
        rx: &mut watch::Receiver<PlayerSnapshot>,
        from_seq: u64,
    ) -> PlayerSnapshot {
        loop {
            rx.changed().await.expect("actor alive");
            let snapshot = rx.borrow().clone();
            if snapshot.seq > from_seq {
                return snapshot;
            }
        }
    }

    #[test]
    fn frame_clock_math() {
        let clock = FrameClock::new();
        assert_eq!(clock.position_ms(), 0);

        clock.reset(48_000);
        clock.advance(48_000);
        assert_eq!(clock.position_ms(), 1000);

        let epoch = clock.epoch();
        clock.seek(Duration::from_secs(2));
        assert_eq!(clock.position_ms(), 2000);
        assert_eq!(clock.epoch(), epoch + 1);

        // A device change advances the epoch but leaves the timeline untouched.
        let frames = clock.frames();
        let epoch = clock.epoch();
        clock.mark_device_change();
        assert_eq!(clock.frames(), frames);
        assert_eq!(clock.epoch(), epoch + 1);
    }

    #[tokio::test]
    async fn lifecycle_transitions() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 1000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        assert_eq!(loading.state, PlaybackState::Loading);
        assert_eq!(loading.duration_ms, Some(1000));

        player
            .engine_now(loaded(44_100, PositionDrive::Frames))
            .await;
        let playing = next_transition(&mut rx, loading.seq).await;
        assert_eq!(playing.state, PlaybackState::Playing);
        assert!(playing.rate > 0.5);

        player.command(Command::Pause).await.unwrap();
        let paused = next_transition(&mut rx, playing.seq).await;
        assert_eq!(paused.state, PlaybackState::Paused);
        assert!(paused.rate < 0.5);

        player.command(Command::Play).await.unwrap();
        let resumed = next_transition(&mut rx, paused.seq).await;
        assert_eq!(resumed.state, PlaybackState::Playing);

        player.command(Command::Stop).await.unwrap();
        let stopped = next_transition(&mut rx, resumed.seq).await;
        assert_eq!(stopped.state, PlaybackState::Idle);
        assert!(stopped.track.is_none());
        assert_eq!(stopped.position_ms, 0);
    }

    /// The tide-2f85 property: reality (the output device) moves, and the emitted view
    /// moves with it — with a *continuous* position, not a frozen or reset one.
    #[tokio::test]
    async fn device_change_reemits_with_continuous_position() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 5000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(48_000, PositionDrive::Frames))
            .await;
        let playing = next_transition(&mut rx, loading.seq).await;
        assert_eq!(playing.state, PlaybackState::Playing);

        // Simulate ~1s of emitted audio, then the output device changes underneath us.
        player.clock().advance(48_000);
        player.engine_now(EngineEvent::DeviceChanged).await;

        let after = next_transition(&mut rx, playing.seq).await;
        assert_eq!(after.state, PlaybackState::Playing); // still playing
        assert!(after.seq > playing.seq); // the emitter moved
        assert!(after.position_ms >= 1000 && after.position_ms < 1100); // continuous
    }

    /// Next effect off the executor channel, failing the test rather than hanging.
    async fn next_effect(effects: &mut mpsc::UnboundedReceiver<Effect>) -> Effect {
        tokio::time::timeout(Duration::from_secs(1), effects.recv())
            .await
            .expect("an effect in time")
            .expect("actor alive")
    }

    fn started(effect: &Effect) -> (u64, String, Duration) {
        match effect {
            Effect::Start {
                generation,
                track,
                position,
            } => (*generation, track.meta.title.clone(), *position),
            other => panic!("expected a start, got {other:?}"),
        }
    }

    /// A dead network sink moves the output back to local and resumes there, from where the
    /// listener was: a fresh start, not a player left claiming to play on nothing.
    #[tokio::test]
    async fn sink_failure_falls_back_to_local_and_resumes() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 60_000)))
            .await
            .unwrap();
        let (generation, _, _) = started(&next_effect(&mut effects).await);
        player
            .engine(generation, loaded(48_000, PositionDrive::Frames))
            .await;
        player
            .command(Command::SelectSink(SinkId("cast-1".to_string())))
            .await
            .unwrap();
        next_effect(&mut effects).await; // the restart onto the renderer
        player.clock().advance(5 * 48_000);

        let before = rx.borrow().seq;
        player.sink_failed(SinkId("cast-1".to_string())).await;
        let (_, title, position) = started(&next_effect(&mut effects).await);
        assert_eq!(title, "t");
        let recovered = next_transition(&mut rx, before).await;
        assert_eq!(recovered.sink, Some(SinkInfo::local().id));
        assert_eq!(
            recovered.state,
            PlaybackState::Loading,
            "restarting, not wedged"
        );
        assert!(position >= Duration::ZERO);
    }

    /// The queue is actor state: growing it is a transition clients see, and its contents are
    /// published for them.
    #[tokio::test]
    async fn enqueueing_while_playing_is_a_transition() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let mut rx = player.subscribe();

        player
            .command(Command::Enqueue(track("a", 1_000)))
            .await
            .unwrap();
        let (generation, title, _) = started(&next_effect(&mut effects).await);
        assert_eq!(title, "a", "enqueueing into an idle player starts it");
        player
            .engine(generation, loaded(44_100, PositionDrive::Frames))
            .await;
        let playing = next_transition(&mut rx, 0).await;

        player
            .command(Command::Enqueue(track("b", 1_000)))
            .await
            .unwrap();
        let grown = next_transition(&mut rx, playing.seq).await;
        assert_eq!(grown.queue.len, 2);
        assert!(grown.queue.revision > playing.queue.revision);
        assert_eq!(
            grown.state,
            PlaybackState::Playing,
            "the new entry waits its turn"
        );
        let titles: Vec<String> = player
            .queue()
            .tracks
            .iter()
            .map(|t| t.meta.title.clone())
            .collect();
        assert_eq!(titles, ["a", "b"]);
        assert!(effects.try_recv().is_err(), "nothing new to start");
    }

    /// The queue advances inside the actor, on the playback's own end, and stops at the end of
    /// the queue.
    #[tokio::test]
    async fn the_end_of_a_track_advances_the_queue() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let mut rx = player.subscribe();

        player
            .command(Command::Enqueue(track("a", 1_000)))
            .await
            .unwrap();
        player
            .command(Command::Enqueue(track("b", 1_000)))
            .await
            .unwrap();
        let (first, _, _) = started(&next_effect(&mut effects).await);

        player.engine(first, EngineEvent::Ended).await;
        let (second, title, position) = started(&next_effect(&mut effects).await);
        assert_eq!((title.as_str(), position), ("b", Duration::ZERO));
        assert!(second > first);

        player.engine(second, EngineEvent::Ended).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while rx.borrow().state != PlaybackState::Ended {
                rx.changed().await.expect("actor alive");
            }
        })
        .await
        .expect("the queue ends");
        assert_eq!(rx.borrow().queue.index, 1);
    }

    /// canon-587a, now in the one place it can be judged: the user skips, and the skipped
    /// track's end arrives afterwards. It belongs to a playback we left; it must not advance
    /// the queue a second time.
    #[tokio::test]
    async fn an_end_from_a_playback_we_left_is_ignored() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();

        for title in ["a", "b", "c"] {
            player
                .command(Command::Enqueue(track(title, 1_000)))
                .await
                .unwrap();
        }
        let (first, _, _) = started(&next_effect(&mut effects).await);
        player.command(Command::Next).await.unwrap();
        let (second, title, _) = started(&next_effect(&mut effects).await);
        assert_eq!(title, "b");

        player.engine(first, EngineEvent::Ended).await; // late news about "a"
        player.command(Command::SetMuted(true)).await.unwrap(); // flush the actor
        assert_eq!(next_effect(&mut effects).await, Effect::SetMuted(true));
        assert_eq!(player.snapshot().queue.index, 1, "still on b");
        assert!(second > first);
    }

    /// The actor knows its own state, so it is the one to refuse what cannot be done.
    #[tokio::test]
    async fn impossible_commands_are_refused() {
        let player = PlayerHandle::spawn();
        assert!(player.command(Command::Next).await.is_err());
        assert!(player.command(Command::Previous).await.is_err());
        assert!(
            player
                .command(Command::Seek(Duration::from_secs(5)))
                .await
                .is_err()
        );
        player
            .command(Command::Load(track("only", 1_000)))
            .await
            .unwrap();
        assert!(
            player.command(Command::Next).await.is_err(),
            "no next track"
        );
    }

    /// A command that changes nothing is accepted and is not news.
    #[tokio::test]
    async fn a_command_that_changes_nothing_is_not_a_transition() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let mut rx = player.subscribe();
        player.command(Command::Pause).await.unwrap(); // nothing is playing
        player.command(Command::Stop).await.unwrap(); // already idle
        player.command(Command::SetVolume(1.0)).await.unwrap(); // already there
        player.command(Command::SetMuted(true)).await.unwrap();
        let muted = next_transition(&mut rx, 0).await;
        assert_eq!(muted.seq, 1, "only the mute was a transition");
        assert_eq!(next_effect(&mut effects).await, Effect::SetMuted(true));
    }

    /// Stopping halts the output and keeps the queue, so it can be picked up again.
    #[tokio::test]
    async fn stop_halts_and_keeps_the_queue() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        player
            .command(Command::Enqueue(track("a", 1_000)))
            .await
            .unwrap();
        let (first, _, _) = started(&next_effect(&mut effects).await);
        player.command(Command::Stop).await.unwrap();
        match next_effect(&mut effects).await {
            Effect::Halt { generation } => assert!(generation > first),
            other => panic!("expected a halt, got {other:?}"),
        }
        assert_eq!(player.snapshot().state, PlaybackState::Idle);
        assert_eq!(player.queue().tracks.len(), 1);
    }

    /// Metadata the source resolves at start is kept on the queue entry.
    #[tokio::test]
    async fn resolved_metadata_is_written_back_into_the_queue() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let bare = TrackRef {
            id: EntityId::new(),
            meta: TrackMeta::default(),
            sources: vec![],
        };
        player.command(Command::Load(bare)).await.unwrap();
        let (generation, _, _) = started(&next_effect(&mut effects).await);
        let meta = TrackMeta {
            title: "Army of Me".into(),
            duration_ms: Some(234_000),
            ..TrackMeta::default()
        };
        player
            .engine(generation, EngineEvent::Described(meta))
            .await;
        player.command(Command::SetMuted(true)).await.unwrap(); // flush
        assert_eq!(player.queue().tracks[0].meta.title, "Army of Me");
        let snapshot = player.snapshot();
        assert_eq!(snapshot.duration_ms, Some(234_000));
        assert_eq!(snapshot.track.unwrap().meta.title, "Army of Me");
    }

    /// Opening a stream on a renderer is not playback: the device has a URL and is filling
    /// its buffer. Claiming `Playing` here is what makes position run ahead of the audio.
    #[tokio::test]
    async fn a_renderer_stream_is_loading_until_the_device_says_otherwise() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(44_100, PositionDrive::Renderer))
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;
        assert_eq!(opened.state, PlaybackState::Loading);

        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        let playing = next_transition(&mut rx, opened.seq).await;
        assert_eq!(playing.state, PlaybackState::Playing);
    }

    /// The heart of the fix: frames fed to the encoder race ahead of the listener, so on a
    /// renderer they must not be position. Only the device's own report moves it.
    #[tokio::test]
    async fn frames_fed_are_not_position_on_a_renderer() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(48_000, PositionDrive::Renderer))
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;

        // The encoder has run 4s ahead of realtime, as the pacing loop is designed to.
        player.clock().advance(4 * 48_000);
        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        let playing = next_transition(&mut rx, opened.seq).await;
        assert!(
            playing.position_ms < 500,
            "position followed frames fed ({}ms) instead of the renderer",
            playing.position_ms
        );

        // The device says where it really is, and that is what the snapshot reports.
        player
            .engine_now(EngineEvent::RendererPosition(Duration::from_secs(30)))
            .await;
        rx.changed().await.expect("actor alive");
        let reconciled = rx.borrow().clone();
        assert!(
            reconciled.position_ms >= 29_000,
            "expected to snap to the device, got {}ms",
            reconciled.position_ms
        );
    }

    /// A correction is not a seek. Clients treat a new `seq` as "something happened"; routine
    /// reconciliation happens twice a second and must not look like the user jumping around.
    #[tokio::test]
    async fn reconciling_position_is_not_a_transition() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(44_100, PositionDrive::Renderer))
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;
        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        let playing = next_transition(&mut rx, opened.seq).await;

        for tick in 1..=4 {
            player
                .engine_now(EngineEvent::RendererPosition(Duration::from_millis(
                    tick * 500,
                )))
                .await;
        }
        // Flush: a later transition must still be the *next* seq, proving none of the
        // reports in between counted as one.
        player.command(Command::Pause).await.unwrap();
        let paused = next_transition(&mut rx, playing.seq).await;
        assert_eq!(
            paused.seq,
            playing.seq + 1,
            "position reports consumed {} transition(s)",
            paused.seq - playing.seq - 1
        );
    }

    /// A renderer restates its condition on every poll, and those reports have to keep
    /// arriving (suppressing them upstream is what once wedged playback in `Loading` after a
    /// seek). The player absorbs the repeats: it re-reports the condition, and only a genuine
    /// change counts as a transition.
    #[tokio::test]
    async fn a_restated_renderer_condition_is_absorbed_not_a_transition() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(44_100, PositionDrive::Renderer))
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;
        assert_eq!(opened.state, PlaybackState::Loading);

        // The device has been saying "playing" all along; the first one we act on lifts us
        // out of Loading even though it is not the device's first such report.
        for _ in 0..5 {
            player
                .engine_now(EngineEvent::RendererState(RendererState::Playing))
                .await;
        }
        let playing = next_transition(&mut rx, opened.seq).await;
        assert_eq!(playing.state, PlaybackState::Playing);

        player.command(Command::Stop).await.unwrap();
        let stopped = next_transition(&mut rx, playing.seq).await;
        assert_eq!(
            stopped.seq,
            playing.seq + 1,
            "restated conditions consumed {} transition(s)",
            stopped.seq - playing.seq - 1
        );
    }

    /// A renderer times the stream it was handed, so after a seek its reports start near
    /// zero. Reading those as absolute collapsed the position to the top of the track.
    #[tokio::test]
    async fn a_renderer_report_is_relative_to_the_stream_it_was_given() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        // Seeking restarts the stream at the seek point: this one begins at 0:47.
        player
            .engine_now(EngineEvent::Loaded {
                sample_rate: 44_100,
                duration_ms: None,
                start_ms: 47_000,
                drive: PositionDrive::Renderer,
            })
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;
        assert_eq!(opened.position_ms, 47_000);

        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        let playing = next_transition(&mut rx, opened.seq).await;
        // "4 seconds into the stream you gave me" is 0:51 of the track, not 0:04.
        player
            .engine_now(EngineEvent::RendererPosition(Duration::from_secs(4)))
            .await;
        rx.changed().await.expect("actor alive");
        let reconciled = rx.borrow().clone();
        assert_eq!(reconciled.seq, playing.seq);
        assert!(
            reconciled.position_ms >= 50_000,
            "the stream's zero was read as the top of the track: {}ms",
            reconciled.position_ms
        );
    }

    /// Stopping ends the renderer stream. A speaker still playing out its buffer keeps saying
    /// "playing" for a while, and that must not revive a stopped player.
    #[tokio::test]
    async fn a_renderer_report_after_stop_is_not_news() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(44_100, PositionDrive::Renderer))
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;
        player.command(Command::Stop).await.unwrap();
        let stopped = next_transition(&mut rx, opened.seq).await;
        assert_eq!(stopped.state, PlaybackState::Idle);

        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        player
            .engine_now(EngineEvent::RendererPosition(Duration::from_secs(9)))
            .await;
        // Flush with a real transition: it must be the very next seq, and still idle.
        player.command(Command::SetMuted(true)).await.unwrap();
        let after = next_transition(&mut rx, stopped.seq).await;
        assert_eq!(after.seq, stopped.seq + 1);
        assert_eq!(after.state, PlaybackState::Idle);
        assert_eq!(after.position_ms, 0);
    }

    /// Position is extrapolated between renderer reports, and the one that ends the track lags
    /// the audio; the snapshot must never claim a position past the end of the track.
    #[tokio::test]
    async fn position_never_runs_past_the_end_of_the_track() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 10_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(44_100, PositionDrive::Renderer))
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;
        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        next_transition(&mut rx, opened.seq).await;
        player
            .engine_now(EngineEvent::RendererPosition(Duration::from_secs(12)))
            .await;
        rx.changed().await.expect("actor alive");
        assert_eq!(rx.borrow().position_ms, 10_000);
    }

    fn prepared(effect: &Effect) -> (u64, u64, String) {
        match effect {
            Effect::Prepare {
                generation,
                next_generation,
                track,
            } => (*generation, *next_generation, track.meta.title.clone()),
            other => panic!("expected a prepare, got {other:?}"),
        }
    }

    /// Two entries on the local output, the first already within the preload lead of its end.
    async fn near_the_end_of_a(
        player: &PlayerHandle,
        effects: &mut mpsc::UnboundedReceiver<Effect>,
    ) -> u64 {
        player
            .command(Command::Enqueue(track("a", 10_000)))
            .await
            .unwrap();
        player
            .command(Command::Enqueue(track("b", 200_000)))
            .await
            .unwrap();
        let (generation, _, _) = started(&next_effect(effects).await);
        player
            .engine(generation, loaded(48_000, PositionDrive::Frames))
            .await;
        generation
    }

    /// Near the end of a track on the local output, the next entry is made ready — once.
    #[tokio::test]
    async fn the_next_entry_is_prepared_near_the_end_of_a_track() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let current = near_the_end_of_a(&player, &mut effects).await;

        let (generation, next_generation, title) = prepared(&next_effect(&mut effects).await);
        assert_eq!((generation, title.as_str()), (current, "b"));
        assert!(next_generation > current);
        tokio::time::sleep(Duration::from_millis(600)).await; // a couple more ticks
        assert!(
            effects.try_recv().is_err(),
            "prepared once, not on every tick"
        );
    }

    /// The listener crosses into the prepared entry: it becomes the current one with no restart,
    /// and its own end then advances the queue as any playback's would.
    #[tokio::test]
    async fn crossing_into_the_prepared_entry_moves_the_queue_on_without_a_restart() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let mut rx = player.subscribe();
        let current = near_the_end_of_a(&player, &mut effects).await;
        let (_, next, _) = prepared(&next_effect(&mut effects).await);
        let meta = TrackMeta {
            title: "b, described".into(),
            duration_ms: Some(200_000),
            ..TrackMeta::default()
        };
        player.engine(next, EngineEvent::Described(meta)).await;

        let before = rx.borrow().seq;
        player
            .engine(current, EngineEvent::Advanced { to: next })
            .await;
        let crossed = next_transition(&mut rx, before).await;
        assert_eq!(
            crossed.state,
            PlaybackState::Playing,
            "no loading between tracks"
        );
        assert_eq!(crossed.queue.index, 1);
        assert_eq!(crossed.track.unwrap().meta.title, "b, described");
        assert_eq!(crossed.duration_ms, Some(200_000));
        assert!(
            effects.try_recv().is_err(),
            "nothing was started or stopped"
        );

        // The playback is now `next`: its end is the one that counts.
        player.engine(current, EngineEvent::Ended).await; // stale: the old playback
        player.engine(next, EngineEvent::Ended).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while rx.borrow().state != PlaybackState::Ended {
                rx.changed().await.expect("actor alive");
            }
        })
        .await
        .expect("b ends the queue");
    }

    /// A skip while the next entry is prepared abandons the preparation; a late hand-off report
    /// from the old playback changes nothing.
    #[tokio::test]
    async fn a_skip_abandons_the_prepared_entry() {
        let (player, mut effects) = PlayerHandle::spawn_with_effects();
        let current = near_the_end_of_a(&player, &mut effects).await;
        let (_, next, _) = prepared(&next_effect(&mut effects).await);

        player.command(Command::Next).await.unwrap();
        let (started_generation, title, _) = started(&next_effect(&mut effects).await);
        assert_eq!(title, "b");
        assert_ne!(
            started_generation, next,
            "a fresh playback, never the abandoned one's"
        );

        player
            .engine(current, EngineEvent::Advanced { to: next })
            .await;
        player.command(Command::SetMuted(true)).await.unwrap(); // flush
        assert_eq!(player.snapshot().queue.index, 1);
        assert_eq!(next_effect(&mut effects).await, Effect::SetMuted(true));
    }

    /// Open a renderer stream and bring it to playing, for the confirmation-window tests.
    async fn playing_on_a_renderer(
        player: &PlayerHandle,
        rx: &mut watch::Receiver<PlayerSnapshot>,
    ) {
        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        player
            .engine_now(loaded(44_100, PositionDrive::Renderer))
            .await;
        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        while rx.borrow().state != PlaybackState::Playing {
            rx.changed().await.expect("actor alive");
        }
    }

    /// canon-7c17: the poll taken just before the device received our pause still says
    /// "playing". That report is older than the command, and must not flip the state back.
    #[tokio::test(start_paused = true)]
    async fn a_report_older_than_a_pause_does_not_undo_it() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();
        playing_on_a_renderer(&player, &mut rx).await;

        player.command(Command::Pause).await.unwrap();
        let paused = rx.borrow().seq;
        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await; // stale
        player
            .engine_now(EngineEvent::RendererState(RendererState::Paused))
            .await; // the device confirms
        player.command(Command::SetMuted(true)).await.unwrap(); // flush
        let after = player.snapshot();
        assert_eq!(after.state, PlaybackState::Paused);
        assert_eq!(after.seq, paused + 1, "only the mute moved seq; no flicker");
    }

    /// The same on the way back: a stale "paused" after a play is not the device refusing.
    #[tokio::test(start_paused = true)]
    async fn a_report_older_than_a_play_does_not_undo_it() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();
        playing_on_a_renderer(&player, &mut rx).await;
        player.command(Command::Pause).await.unwrap();
        player
            .engine_now(EngineEvent::RendererState(RendererState::Paused))
            .await;

        player.command(Command::Play).await.unwrap();
        player
            .engine_now(EngineEvent::RendererState(RendererState::Paused))
            .await; // stale
        player.command(Command::SetMuted(true)).await.unwrap(); // flush
        assert_eq!(player.snapshot().state, PlaybackState::Playing);
    }

    /// The device stays the authority: one that really did not pause is believed once the window
    /// has passed.
    #[tokio::test(start_paused = true)]
    async fn a_device_that_ignored_the_pause_is_believed_after_the_window() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();
        playing_on_a_renderer(&player, &mut rx).await;
        player.command(Command::Pause).await.unwrap();

        tokio::time::advance(CONFIRM_WINDOW + Duration::from_millis(1)).await;
        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        player.command(Command::SetMuted(true)).await.unwrap(); // flush
        assert_eq!(player.snapshot().state, PlaybackState::Playing);
    }

    /// A renderer that rebuffers mid-track is genuinely not progressing. The position must
    /// freeze where it is rather than free-running (or, worse, resetting to zero).
    #[tokio::test]
    async fn rebuffering_freezes_position_instead_of_zeroing_it() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player
            .command(Command::Load(track("t", 300_000)))
            .await
            .unwrap();
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine_now(loaded(44_100, PositionDrive::Renderer))
            .await;
        let opened = next_transition(&mut rx, loading.seq).await;
        player
            .engine_now(EngineEvent::RendererState(RendererState::Playing))
            .await;
        let playing = next_transition(&mut rx, opened.seq).await;
        player
            .engine_now(EngineEvent::RendererPosition(Duration::from_secs(45)))
            .await;

        player
            .engine_now(EngineEvent::RendererState(RendererState::Buffering))
            .await;
        let buffering = next_transition(&mut rx, playing.seq).await;
        assert_eq!(buffering.state, PlaybackState::Loading);
        assert!(
            buffering.position_ms >= 44_000,
            "rebuffering lost the position: {}ms",
            buffering.position_ms
        );

        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            rx.borrow().position_ms < 46_000,
            "position kept running while the device was buffering"
        );
    }
}
