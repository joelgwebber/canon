//! The player state actor: the single owner of playback state.
//!
//! This is the runtime realisation of the state core. One task owns the transport
//! state and the emit loop; the WebSocket+JSON API and the MCP layer talk to it only
//! through [`Command`]s, and the audio/sink layers feed [`EngineEvent`]s back in.
//! Because *every* input that can change playback reality — including device loss and
//! sink failure — is a message that flows through this one actor and re-emits, the
//! published [`PlayerSnapshot`] can never silently desync from what is actually
//! happening (the class of bug behind tideway tide-2f85).
//!
//! Position is not stored: it is derived from the shared [`FrameClock`] the realtime
//! output callback advances, so a stalled callback shows up as frames that stop
//! advancing rather than an invisibly frozen emitter.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};

use crate::{Command, FrameClock, PlaybackState, PlayerSnapshot, SinkId, TrackRef};

/// How often the actor refreshes derived position while playing. Snapshots also carry
/// a `rate`, so clients interpolate between these and this can stay coarse.
const POSITION_TICK: Duration = Duration::from_millis(250);

/// The id the player routes to when an active network sink fails.
const LOCAL_SINK: &str = "local";

/// Signals the audio/sink layers feed back into the state machine — the "reality
/// changed" inputs. Making these explicit transitions (rather than out-of-band
/// mutations on a side thread) is exactly what keeps the emitted state truthful.
#[derive(Debug, Clone)]
pub enum EngineEvent {
    /// The decoder opened the stream and playback begins. Carries the source sample
    /// rate (to rebase the frame clock), the known duration if any, and the timeline
    /// position this stream starts at (non-zero after a seek), so the player rebases and
    /// positions the clock in one step.
    Loaded {
        sample_rate: u32,
        duration_ms: Option<u64>,
        start_ms: u64,
    },
    /// The decoder reached end of stream.
    Ended,
    /// Unrecoverable playback error.
    Failed(String),
    /// The output device/stream was reopened for the *same* track (device loss +
    /// recovery, sleep/wake). Position continues; only the output identity changed.
    DeviceChanged,
    /// The active network sink died; the player fails back to local output.
    SinkFailed(SinkId),
}

enum Input {
    Command(Command),
    Engine(EngineEvent),
}

/// A cheap, cloneable handle to the running player actor.
#[derive(Clone)]
pub struct PlayerHandle {
    input: mpsc::Sender<Input>,
    snapshots: watch::Receiver<PlayerSnapshot>,
    clock: Arc<FrameClock>,
}

impl PlayerHandle {
    /// Spawn the actor on the current Tokio runtime and return a handle. The actor
    /// runs until every handle is dropped.
    #[must_use]
    pub fn spawn() -> PlayerHandle {
        let clock = Arc::new(FrameClock::new());
        let (input_tx, input_rx) = mpsc::channel(64);
        let (snap_tx, snap_rx) = watch::channel(PlayerSnapshot::idle());
        let actor = Actor::new(clock.clone(), snap_tx);
        tokio::spawn(actor.run(input_rx));
        PlayerHandle {
            input: input_tx,
            snapshots: snap_rx,
            clock,
        }
    }

    /// Send a control-plane command (from the API / MCP layer).
    pub async fn command(&self, cmd: Command) {
        let _ = self.input.send(Input::Command(cmd)).await;
    }

    /// Feed an engine event (from the audio / sink layers).
    pub async fn engine(&self, event: EngineEvent) {
        let _ = self.input.send(Input::Engine(event)).await;
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
    clock: Arc<FrameClock>,
    snap_tx: watch::Sender<PlayerSnapshot>,
    last_emitted_position_ms: u64,
}

impl Actor {
    fn new(clock: Arc<FrameClock>, snap_tx: watch::Sender<PlayerSnapshot>) -> Self {
        Self {
            seq: 0,
            state: PlaybackState::Idle,
            track: None,
            duration_ms: None,
            volume: 1.0,
            muted: false,
            sink: None,
            error: None,
            clock,
            snap_tx,
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
                            self.handle(input);
                            self.emit();
                        }
                        // All handles dropped: nothing can command us again.
                        None => break,
                    }
                }
                _ = tick.tick() => {
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

    fn handle(&mut self, input: Input) {
        match input {
            Input::Command(cmd) => self.handle_command(cmd),
            Input::Engine(event) => self.handle_engine(event),
        }
        // Every discrete input is a transition; clients reconcile by seq.
        self.seq += 1;
    }

    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Load(track) => {
                self.duration_ms = track.meta.duration_ms;
                self.track = Some(track);
                self.error = None;
                self.state = PlaybackState::Loading;
            }
            Command::Play => {
                if self.state == PlaybackState::Paused {
                    self.state = PlaybackState::Playing;
                }
            }
            Command::Pause => {
                if self.state == PlaybackState::Playing {
                    self.state = PlaybackState::Paused;
                }
            }
            Command::Stop => {
                self.state = PlaybackState::Idle;
                self.track = None;
                self.duration_ms = None;
                self.error = None;
                self.clock.reset(0);
            }
            Command::Seek(position) => self.clock.seek(position),
            Command::SetVolume(volume) => self.volume = volume.clamp(0.0, 1.0),
            Command::SetMuted(muted) => self.muted = muted,
            Command::SelectSink(id) => self.sink = Some(id),
            // Queue orchestration is the controller's job; the bare actor ignores it.
            Command::Enqueue(_) | Command::Next | Command::Previous | Command::Clear => {}
        }
    }

    fn handle_engine(&mut self, event: EngineEvent) {
        match event {
            EngineEvent::Loaded {
                sample_rate,
                duration_ms,
                start_ms,
            } => {
                self.clock.reset(sample_rate);
                if start_ms > 0 {
                    self.clock.seek(Duration::from_millis(start_ms));
                }
                if duration_ms.is_some() {
                    self.duration_ms = duration_ms;
                }
                self.error = None;
                self.state = PlaybackState::Playing;
            }
            EngineEvent::Ended => self.state = PlaybackState::Ended,
            EngineEvent::Failed(message) => {
                self.state = PlaybackState::Error;
                self.error = Some(message);
            }
            // Same track continues through a reopened output: only mark the
            // discontinuity, then re-emit so the view tracks reality.
            EngineEvent::DeviceChanged => self.clock.mark_device_change(),
            EngineEvent::SinkFailed(id) => {
                if self.sink.as_ref() == Some(&id) {
                    // Fail back to local; playback is never left wedged.
                    self.sink = Some(SinkId(LOCAL_SINK.to_string()));
                }
            }
        }
    }

    fn position_ms(&self) -> u64 {
        match self.state {
            PlaybackState::Playing | PlaybackState::Paused | PlaybackState::Ended => {
                self.clock.position_ms()
            }
            PlaybackState::Idle | PlaybackState::Loading | PlaybackState::Error => 0,
        }
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
        }
    }

    fn emit(&mut self) {
        let snapshot = self.snapshot();
        self.last_emitted_position_ms = snapshot.position_ms;
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

        player.command(Command::Load(track("t", 1000))).await;
        let loading = next_transition(&mut rx, 0).await;
        assert_eq!(loading.state, PlaybackState::Loading);
        assert_eq!(loading.duration_ms, Some(1000));

        player
            .engine(EngineEvent::Loaded {
                sample_rate: 44_100,
                duration_ms: None,
                start_ms: 0,
            })
            .await;
        let playing = next_transition(&mut rx, loading.seq).await;
        assert_eq!(playing.state, PlaybackState::Playing);
        assert!(playing.rate > 0.5);

        player.command(Command::Pause).await;
        let paused = next_transition(&mut rx, playing.seq).await;
        assert_eq!(paused.state, PlaybackState::Paused);
        assert!(paused.rate < 0.5);

        player.command(Command::Play).await;
        let resumed = next_transition(&mut rx, paused.seq).await;
        assert_eq!(resumed.state, PlaybackState::Playing);

        player.command(Command::Stop).await;
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

        player.command(Command::Load(track("t", 5000))).await;
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine(EngineEvent::Loaded {
                sample_rate: 48_000,
                duration_ms: None,
                start_ms: 0,
            })
            .await;
        let playing = next_transition(&mut rx, loading.seq).await;
        assert_eq!(playing.state, PlaybackState::Playing);

        // Simulate ~1s of emitted audio, then the output device changes underneath us.
        player.clock().advance(48_000);
        player.engine(EngineEvent::DeviceChanged).await;

        let after = next_transition(&mut rx, playing.seq).await;
        assert_eq!(after.state, PlaybackState::Playing); // still playing
        assert!(after.seq > playing.seq); // the emitter moved
        assert!(after.position_ms >= 1000 && after.position_ms < 1100); // continuous
    }

    #[tokio::test]
    async fn sink_failure_falls_back_to_local() {
        let player = PlayerHandle::spawn();
        let mut rx = player.subscribe();

        player.command(Command::Load(track("t", 1000))).await;
        let loading = next_transition(&mut rx, 0).await;
        player
            .engine(EngineEvent::Loaded {
                sample_rate: 44_100,
                duration_ms: None,
                start_ms: 0,
            })
            .await;
        let playing = next_transition(&mut rx, loading.seq).await;

        player
            .command(Command::SelectSink(SinkId("cast-1".to_string())))
            .await;
        let casting = next_transition(&mut rx, playing.seq).await;
        assert_eq!(casting.sink, Some(SinkId("cast-1".to_string())));

        player
            .engine(EngineEvent::SinkFailed(SinkId("cast-1".to_string())))
            .await;
        let recovered = next_transition(&mut rx, casting.seq).await;
        assert_eq!(recovered.state, PlaybackState::Playing); // not wedged
        assert_eq!(recovered.sink, Some(SinkId(LOCAL_SINK.to_string())));
    }
}
