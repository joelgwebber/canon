//! A player as the server keeps it (canon-806d): its actor, and whichever executor carries out its
//! playback.
//!
//! The actor decides; an executor produces the sound. The two meet only through
//! [`ExecutorOrder`]s and [`ExecutorReport`]s, so an executor can be the server's own controller
//! (in process, over a channel) or a player across the network (over its WebSocket) and the actor
//! cannot tell. One executor at a time: a new one supersedes the last, which hears its orders end
//! and goes quiet; reports from a superseded executor are dropped, so its late teardown can't be
//! taken for the current one's.
//!
//! When the executor goes away, nothing is producing the sound any more, and the actor hears so
//! ([`EngineEvent::ExecutorLost`]): playback stops where it was and the next play resumes there.
//! A start with no executor attached fails the same way, at once, rather than sitting in
//! `Loading` waiting for audio nothing will produce.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    Command, ControlPlane, Effect, EntityId, Error, ExecutorOrder, ExecutorReport, PlayerHandle,
    PlayerSnapshot, QueueSnapshot, Result, SinkInfo,
};

/// How long an executor may take to connect to an output it was asked to select. Generous: a
/// renderer gets several attempts to settle (`canon-daemon`'s `connect_settled`).
const SELECT_TIMEOUT: Duration = Duration::from_secs(60);

/// The path, with its token, an executor fetches a track's audio from (`/audio/<id>?token=…`).
pub type AudioUrls = Box<dyn Fn(EntityId) -> String + Send + Sync>;

/// A player: its actor, and the executor carrying out its playback, if one is attached.
pub struct Station {
    shared: Arc<Shared>,
}

struct Shared {
    player: PlayerHandle,
    executor: Mutex<Option<Current>>,
    epochs: AtomicU64,
    requests: AtomicU64,
    /// Output selections an executor has been asked for and hasn't answered.
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<()>>>>,
    /// The outputs the current executor can reach.
    outputs: watch::Sender<Vec<SinkInfo>>,
    audio_urls: Option<AudioUrls>,
}

struct Current {
    epoch: u64,
    orders: mpsc::UnboundedSender<ExecutorOrder>,
}

impl Station {
    /// The station for `player`, whose effects arrive on `effects`. Starts and prepares carry an
    /// audio path from `audio_urls` when given, for an executor that fetches from the server.
    #[must_use]
    pub fn new(
        player: PlayerHandle,
        mut effects: mpsc::UnboundedReceiver<Effect>,
        audio_urls: Option<AudioUrls>,
    ) -> Self {
        let shared = Arc::new(Shared {
            player,
            executor: Mutex::new(None),
            epochs: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            outputs: watch::Sender::new(vec![SinkInfo::local()]),
            audio_urls,
        });
        let forwarder = Arc::clone(&shared);
        tokio::spawn(async move {
            while let Some(effect) = effects.recv().await {
                forwarder.forward(effect).await;
            }
        });
        Self { shared }
    }

    /// The player's actor.
    #[must_use]
    pub fn player(&self) -> &PlayerHandle {
        &self.shared.player
    }

    /// Whether an executor is attached.
    #[must_use]
    pub fn has_executor(&self) -> bool {
        lock(&self.shared.executor).is_some()
    }
}

impl Shared {
    /// Send `effect` to the executor; a start with none to send it to fails at once.
    async fn forward(&self, effect: Effect) {
        let audio = match &effect {
            Effect::Start { track, .. } | Effect::Prepare { track, .. } => {
                self.audio_urls.as_ref().map(|urls| urls(track.id))
            }
            _ => None,
        };
        let start = matches!(effect, Effect::Start { .. });
        let sent = lock(&self.executor).as_ref().is_some_and(|current| {
            current
                .orders
                .send(ExecutorOrder::Effect { effect, audio })
                .is_ok()
        });
        if !sent && start {
            self.player
                .executor_lost("no player is connected to play it".into())
                .await;
        }
    }

    fn is_current(&self, epoch: u64) -> bool {
        lock(&self.executor)
            .as_ref()
            .is_some_and(|current| current.epoch == epoch)
    }
}

/// An attached executor's side of a [`Station`]: where its reports go. Dropping it detaches the
/// executor, if it is still the current one.
pub struct ExecutorLink {
    shared: Arc<Shared>,
    epoch: u64,
}

impl ExecutorLink {
    /// Take one report from this executor. Dropped if another executor has superseded it.
    pub async fn report(&self, report: ExecutorReport) {
        if !self.shared.is_current(self.epoch) {
            return;
        }
        match report {
            ExecutorReport::Engine { generation, event } => {
                self.shared.player.report(generation, event).await;
            }
            ExecutorReport::Outputs { sinks } => {
                self.shared.outputs.send_replace(sinks);
            }
            ExecutorReport::Selected { request, error } => {
                if let Some(reply) = lock(&self.shared.pending).remove(&request) {
                    let _ = reply.send(error.map_or(Ok(()), |why| Err(Error::Sink(why))));
                }
            }
        }
    }
}

impl Drop for ExecutorLink {
    fn drop(&mut self) {
        let was_current = {
            let mut executor = lock(&self.shared.executor);
            let current = executor.as_ref().is_some_and(|c| c.epoch == self.epoch);
            if current {
                executor.take();
            }
            current
        };
        if !was_current {
            return;
        }
        self.shared.outputs.send_replace(vec![SinkInfo::local()]);
        lock(&self.shared.pending).clear();
        let player = self.shared.player.clone();
        // Drop can't wait; the actor hears it as soon as it can. Without a runtime (a test's end,
        // a process exiting) there is no actor left to tell.
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                player
                    .executor_lost("the player carrying out playback disconnected".into())
                    .await;
            });
        }
    }
}

#[async_trait]
impl ControlPlane for Station {
    /// Selecting an output asks the executor to connect to it first; only once it has does the
    /// actor restart playback there. Everything else is the actor's to decide.
    async fn dispatch(&self, command: Command) -> Result<()> {
        let Command::SelectSink(sink) = command else {
            return self.shared.player.command(command).await;
        };
        let request = self.shared.requests.fetch_add(1, Ordering::Relaxed) + 1;
        let (reply, answer) = oneshot::channel();
        lock(&self.shared.pending).insert(request, reply);
        let asked = lock(&self.shared.executor).as_ref().is_some_and(|current| {
            current
                .orders
                .send(ExecutorOrder::Select {
                    request,
                    sink: sink.clone(),
                })
                .is_ok()
        });
        if !asked {
            lock(&self.shared.pending).remove(&request);
            return Err(Error::Unsupported(
                "no player is connected to play on any output".into(),
            ));
        }
        match tokio::time::timeout(SELECT_TIMEOUT, answer).await {
            Ok(Ok(Ok(()))) => self.shared.player.command(Command::SelectSink(sink)).await,
            Ok(Ok(Err(e))) => Err(e),
            Ok(Err(_)) => Err(Error::Sink(
                "the player carrying out playback went away".into(),
            )),
            Err(_) => {
                lock(&self.shared.pending).remove(&request);
                Err(Error::Sink(format!("{sink:?} didn't answer in time")))
            }
        }
    }

    fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        self.shared.player.subscribe()
    }

    fn snapshot(&self) -> PlayerSnapshot {
        self.shared.player.snapshot()
    }

    fn queue(&self) -> QueueSnapshot {
        self.shared.player.queue()
    }

    fn sinks(&self) -> Vec<SinkInfo> {
        self.shared.outputs.borrow().clone()
    }

    /// Attach an executor, superseding any other: its orders, and where it reports.
    fn attach_executor(&self) -> Result<(mpsc::UnboundedReceiver<ExecutorOrder>, ExecutorLink)> {
        let epoch = self.shared.epochs.fetch_add(1, Ordering::Relaxed) + 1;
        let (orders, received) = mpsc::unbounded_channel();
        // Bring it up to what the player says: a fresh engine starts at full gain.
        let snapshot = self.shared.player.snapshot();
        let _ = orders.send(ExecutorOrder::Effect {
            effect: Effect::SetVolume(snapshot.volume),
            audio: None,
        });
        let _ = orders.send(ExecutorOrder::Effect {
            effect: Effect::SetMuted(snapshot.muted),
            audio: None,
        });
        let superseded = lock(&self.shared.executor)
            .replace(Current { epoch, orders })
            .is_some();
        if superseded {
            // The last executor hears its orders end and goes quiet, mid-track or not.
            lock(&self.shared.pending).clear();
            self.shared.outputs.send_replace(vec![SinkInfo::local()]);
            let player = self.shared.player.clone();
            tokio::spawn(async move {
                player
                    .executor_lost("another player took over playback".into())
                    .await;
            });
        }
        Ok((
            received,
            ExecutorLink {
                shared: Arc::clone(&self.shared),
                epoch,
            },
        ))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EngineEvent, PlaybackState, PositionDrive, SinkId, TrackMeta, TrackRef};

    fn track(title: &str) -> TrackRef {
        TrackRef {
            id: EntityId::new(),
            meta: TrackMeta {
                title: title.into(),
                duration_ms: Some(120_000),
                ..TrackMeta::default()
            },
            sources: Vec::new(),
        }
    }

    fn station() -> Station {
        let (player, effects) = PlayerHandle::spawn_with_effects();
        Station::new(
            player,
            effects,
            Some(Box::new(|id| format!("/audio/{}?token=t", id.0))),
        )
    }

    async fn next(orders: &mut mpsc::UnboundedReceiver<ExecutorOrder>) -> ExecutorOrder {
        tokio::time::timeout(Duration::from_secs(1), orders.recv())
            .await
            .expect("an order in time")
            .expect("orders open")
    }

    async fn settled(station: &Station, state: PlaybackState) -> PlayerSnapshot {
        for _ in 0..100 {
            let snapshot = station.snapshot();
            if snapshot.state == state {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("never {state:?}: {:?}", station.snapshot());
    }

    /// An executor gets the volume to come up at, then the actor's effects with their audio path;
    /// its reports drive the actor exactly as an in-process engine's do.
    #[tokio::test]
    async fn an_executor_carries_out_the_actors_decisions() {
        let station = station();
        let (mut orders, link) = station.attach_executor().unwrap();
        assert!(matches!(
            next(&mut orders).await,
            ExecutorOrder::Effect {
                effect: Effect::SetVolume(_),
                ..
            }
        ));
        assert!(matches!(
            next(&mut orders).await,
            ExecutorOrder::Effect {
                effect: Effect::SetMuted(false),
                ..
            }
        ));

        let army = track("Army of Me");
        station.dispatch(Command::Load(army.clone())).await.unwrap();
        let ExecutorOrder::Effect {
            effect: Effect::Start { generation, .. },
            audio,
        } = next(&mut orders).await
        else {
            panic!("a start");
        };
        assert_eq!(audio, Some(format!("/audio/{}?token=t", army.id.0)));

        link.report(ExecutorReport::Engine {
            generation: Some(generation),
            event: EngineEvent::Loaded {
                sample_rate: 44_100,
                duration_ms: Some(120_000),
                start_ms: 0,
                drive: PositionDrive::Renderer,
                joins: false,
            },
        })
        .await;
        link.report(ExecutorReport::Engine {
            generation: Some(generation),
            event: EngineEvent::RendererState(crate::RendererState::Playing),
        })
        .await;
        settled(&station, PlaybackState::Playing).await;

        link.report(ExecutorReport::Outputs {
            sinks: vec![SinkInfo::local(), kef()],
        })
        .await;
        assert_eq!(station.sinks().len(), 2);
    }

    fn kef() -> SinkInfo {
        SinkInfo {
            id: SinkId("uuid:kef".into()),
            name: "Tunes".into(),
            kind: crate::SinkKind::Dlna,
            protocols: Vec::new(),
        }
    }

    /// Selecting an output waits for the executor to reach it; a refusal is the command's error
    /// and moves nothing.
    #[tokio::test]
    async fn selecting_an_output_asks_the_executor_first() {
        let station = Arc::new(station());
        let (mut orders, link) = station.attach_executor().unwrap();
        next(&mut orders).await;
        next(&mut orders).await;

        let asking = Arc::clone(&station);
        let selecting =
            tokio::spawn(async move { asking.dispatch(Command::SelectSink(kef().id)).await });
        let ExecutorOrder::Select { request, sink } = next(&mut orders).await else {
            panic!("a select");
        };
        assert_eq!(sink, kef().id);
        link.report(ExecutorReport::Selected {
            request,
            error: Some("refused".into()),
        })
        .await;
        assert!(selecting.await.unwrap().is_err());
        assert_eq!(station.snapshot().sink, None, "nothing moved");

        let asking = Arc::clone(&station);
        let selecting =
            tokio::spawn(async move { asking.dispatch(Command::SelectSink(kef().id)).await });
        let ExecutorOrder::Select { request, .. } = next(&mut orders).await else {
            panic!("a select");
        };
        link.report(ExecutorReport::Selected {
            request,
            error: None,
        })
        .await;
        selecting.await.unwrap().unwrap();
        assert_eq!(station.snapshot().sink, Some(kef().id));
    }

    /// With no executor, a start fails at once, keeping the queue; selecting an output is refused.
    #[tokio::test]
    async fn with_no_executor_nothing_waits_for_sound() {
        let station = station();
        station.dispatch(Command::Load(track("a"))).await.unwrap();
        let snapshot = settled(&station, PlaybackState::Idle).await;
        assert!(snapshot.error.unwrap().contains("no player is connected"));
        assert_eq!(snapshot.queue.len, 1);
        assert!(
            station
                .dispatch(Command::SelectSink(kef().id))
                .await
                .is_err()
        );
    }

    /// The executor going away stops playback where it was; a superseded executor's reports are
    /// dropped, and its leaving doesn't count.
    #[tokio::test]
    async fn the_current_executor_leaving_stops_playback_a_superseded_one_does_not() {
        let station = station();
        let (mut first, first_link) = station.attach_executor().unwrap();
        next(&mut first).await;
        next(&mut first).await;
        station.dispatch(Command::Load(track("a"))).await.unwrap();
        let ExecutorOrder::Effect {
            effect: Effect::Start { generation, .. },
            ..
        } = next(&mut first).await
        else {
            panic!("a start");
        };
        first_link
            .report(ExecutorReport::Engine {
                generation: Some(generation),
                event: EngineEvent::Loaded {
                    sample_rate: 44_100,
                    duration_ms: Some(120_000),
                    start_ms: 0,
                    drive: PositionDrive::Frames,
                    joins: true,
                },
            })
            .await;
        settled(&station, PlaybackState::Playing).await;

        // A second executor takes over: the first's playback stops, and its orders end.
        let (_second, second_link) = station.attach_executor().unwrap();
        let snapshot = settled(&station, PlaybackState::Idle).await;
        assert!(snapshot.error.unwrap().contains("took over"));
        while first.recv().await.is_some() {}
        first_link
            .report(ExecutorReport::Outputs {
                sinks: vec![SinkInfo::local(), kef()],
            })
            .await;
        assert_eq!(
            station.sinks().len(),
            1,
            "a superseded executor's report is dropped"
        );
        drop(first_link);
        assert!(
            station.has_executor(),
            "its leaving doesn't detach the current one"
        );

        drop(second_link);
        assert!(!station.has_executor());
    }
}
