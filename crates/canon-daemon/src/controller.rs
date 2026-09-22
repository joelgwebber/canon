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

use std::sync::Arc;

use async_trait::async_trait;
use canon_audio::AudioPlayer;
use canon_core::{
    Codec, Command, ControlPlane, EngineEvent, Error, PlayerHandle, PlayerSnapshot, Quality,
    QueueView, Result, Source, SourceRef, TrackRef,
};
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
    me: std::sync::Weak<Self>,
}

impl PlaybackController {
    pub fn new(player: PlayerHandle, session: Arc<TidalSession>, quality: Quality) -> Arc<Self> {
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

        let audio = AudioPlayer::start(
            resolved.input,
            hint,
            self.player.clock(),
            events_tx,
            start_ms,
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
                let mut inner = self.inner.lock().await;
                if let Some(previous) = inner.audio.take() {
                    previous.stop();
                }
                inner.queue.clear();
                inner.index = 0;
                inner.active = false;
                inner.generation += 1; // invalidate any in-flight start
                drop(inner);
                self.player.command(Command::Stop).await;
                Ok(())
            }
            Command::Play => {
                self.with_audio(&*self.inner.lock().await, AudioPlayer::resume);
                self.player.command(Command::Play).await;
                Ok(())
            }
            Command::Pause => {
                self.with_audio(&*self.inner.lock().await, AudioPlayer::pause);
                self.player.command(Command::Pause).await;
                Ok(())
            }
            Command::Stop => {
                {
                    let mut inner = self.inner.lock().await;
                    if let Some(previous) = inner.audio.take() {
                        previous.stop();
                    }
                    inner.active = false;
                    inner.generation += 1;
                }
                self.player.command(Command::Stop).await;
                Ok(())
            }
            Command::SetVolume(volume) => {
                self.with_audio(&*self.inner.lock().await, |audio| audio.set_volume(volume));
                self.player.command(Command::SetVolume(volume)).await;
                Ok(())
            }
            Command::SetMuted(muted) => {
                self.with_audio(&*self.inner.lock().await, |audio| audio.set_muted(muted));
                self.player.command(Command::SetMuted(muted)).await;
                Ok(())
            }
            Command::SelectSink(id) => {
                self.player.command(Command::SelectSink(id)).await;
                Ok(())
            }
            Command::Seek(position) => self.seek(position).await,
        }
    }

    fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        self.snapshots.subscribe()
    }

    fn snapshot(&self) -> PlayerSnapshot {
        self.snapshots.borrow().clone()
    }
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
