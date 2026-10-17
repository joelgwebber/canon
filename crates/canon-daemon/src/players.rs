//! Every player the server keeps (canon-973d): one player actor, playback controller and autoplay
//! per [`PlayerId`], made the first time a connection names it, and its queue kept in the library
//! so a restart doesn't empty it.
//!
//! What players share — the sources and their logins, the settings, discovery, the one LAN stream
//! server — is built once in `run_serve` and handed to each. What is a player's own is its actor: its
//! queue, its playback, its snapshot.
//!
//! Each player is a [`Station`]: its actor, and an executor carrying out its playback (canon-806d).
//! Every player starts with this server's own controller as its executor, playing to this
//! machine's output and its LAN speakers as canon always has. A player process that connects as an
//! executor (`/ws?player=<id>&role=executor`) takes over from it; when that one leaves, the player
//! has no executor until another connects, and stops where it was rather than starting sound here.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canon_core::{
    Command, ControlPlane, EntityId, PlaybackState, PlayerHandle, PlayerId, PlayerSnapshot,
    Players, Quality, QueueSnapshot, Result, SettingsStore, Sources, Station,
};
use canon_library::{Library, QueuePlace, SavedQueue};
use canon_sink::DiscoveryService;
use tokio::sync::{Mutex, mpsc, watch};
use tokio::task::JoinHandle;

use crate::autoplay;
use crate::controller::{PlaybackController, StreamHost};

/// How often a playing queue's position is saved. A restart resumes at most this far back.
const POSITION_SAVE: Duration = Duration::from_secs(5);

/// What every player is built from.
pub struct Shared {
    pub sources: Sources,
    pub quality: Quality,
    pub settings: Arc<dyn SettingsStore>,
    pub discovery: Option<Arc<DiscoveryService>>,
    pub streams: Arc<StreamHost>,
    pub library: Library,
    /// Signs the audio paths sent with each start, for an executor that fetches from this server.
    pub signer: Option<canon_api::StreamSigner>,
}

/// The server's players, by id.
pub struct PlayerRegistry {
    shared: Shared,
    players: Mutex<HashMap<PlayerId, Kept>>,
}

/// One player and the tasks that run beside it.
struct Kept {
    player: PlayerHandle,
    station: Arc<Station>,
    persister: Persister,
    tasks: Vec<JoinHandle<()>>,
}

impl PlayerRegistry {
    #[must_use]
    pub fn new(shared: Shared) -> Self {
        Self {
            shared,
            players: Mutex::new(HashMap::new()),
        }
    }

    /// Save every player's queue as it stands now, and stop the tasks beside them. For shutdown:
    /// the periodic save may be a few seconds behind.
    pub async fn shut_down(&self) {
        let mut players = self.players.lock().await;
        for (id, kept) in players.drain() {
            for task in kept.tasks {
                task.abort();
            }
            kept.persister.flush(&kept.player).await;
            tracing::debug!(player = %id, "saved its queue");
        }
    }

    /// Build player `id`: its actor and controller, with the queue it last had restored (ready,
    /// not playing), and the tasks beside it.
    async fn spawn(&self, id: &PlayerId) -> Kept {
        let shared = &self.shared;
        let (player, effects) = PlayerHandle::spawn_with_effects();
        let audio_urls = shared.signer.clone().map(|signer| {
            let id = id.clone();
            Box::new(move |track: EntityId| {
                let token = signer.sign(&id, track, canon_api::auth::STREAM_TOKEN_TTL);
                format!("/audio/{}?token={token}", track.0)
            }) as canon_core::AudioUrls
        });
        let station = Arc::new(Station::new(player.clone(), effects, audio_urls));
        let mut tasks = Vec::new();

        // This server's own controller carries out the player's playback until a player process
        // takes over.
        match station.attach_executor() {
            Ok((orders, link)) => {
                let (reports, mut reported) = mpsc::unbounded_channel();
                // Held by the reporting task: the controller's reports flow while it runs.
                let _controller = PlaybackController::new(
                    orders,
                    reports,
                    shared.sources.clone(),
                    shared.quality,
                    Arc::clone(&shared.settings),
                    shared.discovery.clone(),
                    Arc::clone(&shared.streams),
                );
                tasks.push(tokio::spawn(async move {
                    while let Some(report) = reported.recv().await {
                        link.report(report).await;
                    }
                }));
            }
            Err(e) => tracing::warn!(player = %id, "no playback here: {e}"),
        }

        let mut saved = Saved::default();
        match shared.library.player_queue(id.as_str()).await {
            Ok(Some((tracks, place))) if !tracks.is_empty() => {
                saved = Saved {
                    tracks: tracks.iter().map(|track| track.id).collect(),
                    place,
                };
                let restore = Command::Restore {
                    tracks,
                    index: place.index,
                    repeat: place.repeat,
                    position: Duration::from_millis(place.position_ms),
                };
                match player.command(restore).await {
                    Ok(()) => tracing::info!(
                        player = %id,
                        "restored its queue: {} tracks, at {}",
                        saved.tracks.len(),
                        place.index + 1
                    ),
                    Err(e) => tracing::warn!(player = %id, "couldn't restore its queue: {e}"),
                }
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(player = %id, "couldn't read its saved queue: {e}"),
        }

        let persister = Persister {
            id: id.clone(),
            library: shared.library.clone(),
            state: Arc::new(Mutex::new(Persisted::new(saved))),
        };
        tasks.extend([
            tokio::spawn(persister.clone().run(player.clone())),
            autoplay::spawn(
                player.clone(),
                shared.library.clone(),
                shared.sources.clone(),
                Arc::clone(&shared.settings),
            ),
            tokio::spawn(log_snapshots(id.clone(), player.subscribe())),
        ]);
        Kept {
            player,
            station,
            persister,
            tasks,
        }
    }
}

#[async_trait]
impl Players for PlayerRegistry {
    async fn player(&self, id: &PlayerId) -> Result<Arc<dyn ControlPlane>> {
        let mut players = self.players.lock().await;
        if let Some(kept) = players.get(id) {
            return Ok(Arc::clone(&kept.station) as Arc<dyn ControlPlane>);
        }
        tracing::info!(player = %id, "new player");
        let kept = self.spawn(id).await;
        let station = Arc::clone(&kept.station);
        players.insert(id.clone(), kept);
        Ok(station)
    }
}

/// A queue as last written to the library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Saved {
    tracks: Vec<EntityId>,
    place: QueuePlace,
}

/// Keeps one player's saved queue in step with its actor: the tracks whenever they change, where
/// it stands whenever that moves, and the position every few seconds while it plays.
#[derive(Clone)]
struct Persister {
    id: PlayerId,
    library: Library,
    state: Arc<Mutex<Persisted>>,
}

/// What a persister knows: the queue as last written, and where the listener was last heard
/// (see `follow`), which is written only every few seconds.
#[derive(Debug)]
struct Persisted {
    saved: Saved,
    heard_ms: u64,
    /// Nothing has played since the queue was restored, so its restored position is still what
    /// play resumes.
    resting: bool,
}

impl Persisted {
    fn new(saved: Saved) -> Self {
        Self {
            heard_ms: saved.place.position_ms,
            saved,
            resting: true,
        }
    }
}

impl Persister {
    async fn run(self, player: PlayerHandle) {
        let mut snapshots = player.subscribe();
        let mut queues = player.queue_watch();
        let mut tick = tokio::time::interval(POSITION_SAVE);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let timely = tokio::select! {
                changed = snapshots.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    false
                }
                changed = queues.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    false
                }
                _ = tick.tick() => true,
            };
            let snapshot = snapshots.borrow_and_update().clone();
            let queue = queues.borrow_and_update().clone();
            self.save(&queue, &snapshot, timely).await;
        }
    }

    /// Save the queue as it stands now, whatever moved.
    async fn flush(&self, player: &PlayerHandle) {
        self.save(&player.queue(), &player.snapshot(), true).await;
    }

    /// Write what differs from what was last saved: the tracks, if they changed (with the place);
    /// else the place, if the entry or repeat moved, or only the position and `timely`.
    async fn save(&self, queue: &QueueSnapshot, snapshot: &PlayerSnapshot, timely: bool) {
        let mut state = self.state.lock().await;
        state.resting &= snapshot.state == PlaybackState::Idle;
        state.heard_ms = follow(state.heard_ms, snapshot, state.resting);
        let position_ms = state.heard_ms;
        let now = Saved {
            tracks: queue.tracks.iter().map(|track| track.id).collect(),
            place: QueuePlace {
                index: snapshot.queue.index,
                repeat: snapshot.queue.repeat,
                position_ms,
            },
        };
        let saved = &state.saved;
        let written = if now.tracks != saved.tracks {
            let queue = SavedQueue {
                tracks: now.tracks.clone(),
                place: now.place,
            };
            self.library
                .save_player_queue(self.id.as_str(), queue)
                .await
        } else if now.place.index != saved.place.index
            || now.place.repeat != saved.place.repeat
            || (timely && now.place.position_ms != saved.place.position_ms)
        {
            self.library
                .save_player_place(self.id.as_str(), now.place)
                .await
        } else {
            return;
        };
        match written {
            Ok(()) => state.saved = now,
            Err(e) => tracing::warn!(player = %self.id, "couldn't save its queue: {e}"),
        }
    }
}

/// Where the listener is, to save, given where they last were: the snapshot's position while a
/// track plays or is paused; unchanged while one is starting (its position isn't confirmed yet) or
/// while `resting` (a restored queue nothing has played from yet); zero once playback has left a
/// track, which is where play starts from then.
fn follow(position_ms: u64, snapshot: &PlayerSnapshot, resting: bool) -> u64 {
    match snapshot.state {
        PlaybackState::Playing | PlaybackState::Paused => snapshot.position_ms,
        PlaybackState::Loading => position_ms,
        _ if resting => position_ms,
        PlaybackState::Idle | PlaybackState::Ended | PlaybackState::Error => 0,
    }
}

/// Log player `id`'s initial snapshot and every change until its actor stops.
async fn log_snapshots(id: PlayerId, mut snapshots: watch::Receiver<PlayerSnapshot>) {
    loop {
        {
            let snapshot = snapshots.borrow_and_update();
            tracing::info!(
                player = %id,
                seq = snapshot.seq,
                state = ?snapshot.state,
                position_ms = snapshot.position_ms,
                "player state"
            );
        }
        if snapshots.changed().await.is_err() {
            break; // actor gone
        }
    }
}

#[cfg(test)]
mod tests {
    use canon_core::{Repeat, Settings};
    use canon_library::{Store, Track};

    use super::*;

    /// Settings held in memory, autoplay off so nothing but the test touches the queue.
    struct Held;

    #[async_trait]
    impl SettingsStore for Held {
        fn get(&self) -> Settings {
            let mut settings = Settings::default();
            settings.queue.autoplay = false;
            settings
        }
        async fn set(&self, _settings: Settings) -> Result<()> {
            Ok(())
        }
    }

    fn registry(library: &Library) -> PlayerRegistry {
        PlayerRegistry::new(Shared {
            sources: Sources::new(),
            quality: Quality::Lossless,
            settings: Arc::new(Held),
            discovery: None,
            streams: Arc::new(StreamHost::new(0)),
            library: library.clone(),
            signer: None,
        })
    }

    async fn tracks(library: &Library, titles: &[&'static str]) -> Vec<canon_core::TrackRef> {
        let titles = titles.to_vec();
        let ids = library
            .run(move |store| {
                titles
                    .into_iter()
                    .map(|title| {
                        store.add_track(&Track {
                            title: title.into(),
                            credit: "Björk".into(),
                            artists: Vec::new(),
                            duration_ms: Some(240_000),
                            isrcs: Vec::new(),
                            mbid: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .await
            .unwrap();
        library.track_refs(ids).await.unwrap()
    }

    /// Wait until `player`'s saved queue satisfies `done`.
    async fn saved(
        library: &Library,
        player: &str,
        done: impl Fn(&SavedQueue) -> bool,
    ) -> SavedQueue {
        for _ in 0..100 {
            let read = library
                .run({
                    let player = player.to_owned();
                    move |store| store.player_queue(&player)
                })
                .await
                .unwrap();
            if let Some(queue) = read.filter(|queue| done(queue)) {
                return queue;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("{player}'s queue never saved as expected");
    }

    /// A player's queue edits reach the library as they happen, and a fresh registry (a restarted
    /// server) hands the player back its queue, ready and not playing, while another player
    /// starts empty (canon-973d).
    #[tokio::test]
    async fn a_queue_outlives_the_server_that_held_it() {
        let library = Library::new(Store::open_in_memory().unwrap());
        let queued = tracks(&library, &["Army of Me", "Hyperballad", "Isobel"]).await;
        let kitchen = PlayerId::parse("kitchen").unwrap();

        let first = registry(&library);
        let control = first.player(&kitchen).await.unwrap();
        // Queued without starting: no source can play these, and none needs to.
        control
            .dispatch(Command::Restore {
                tracks: queued.clone(),
                index: 0,
                repeat: Repeat::Off,
                position: Duration::ZERO,
            })
            .await
            .unwrap();
        control
            .dispatch(Command::SetRepeat(Repeat::All))
            .await
            .unwrap();
        control
            .dispatch(Command::Move { from: 2, to: 0 })
            .await
            .unwrap();
        let ids: Vec<EntityId> = control.queue().tracks.iter().map(|t| t.id).collect();
        let written = saved(&library, "kitchen", |queue| {
            queue.tracks == ids && queue.place.repeat == Repeat::All
        })
        .await;
        assert_eq!(written.tracks[0], queued[2].id, "the move, as saved");
        first.shut_down().await;

        let second = registry(&library);
        let control = second.player(&kitchen).await.unwrap();
        let snapshot = control.snapshot();
        assert_eq!(snapshot.state, PlaybackState::Idle, "restored, not playing");
        assert_eq!(snapshot.queue.repeat, Repeat::All);
        assert_eq!(snapshot.queue.index, written.place.index);
        let restored: Vec<EntityId> = control.queue().tracks.iter().map(|t| t.id).collect();
        assert_eq!(restored, ids);

        let other = second.player(&PlayerId::default()).await.unwrap();
        assert_eq!(
            other.queue().tracks.len(),
            0,
            "another player's queue is its own"
        );
    }

    /// Where the listener was is what gets saved while a track plays, and is kept, not zeroed,
    /// while a restored queue waits for play.
    #[test]
    fn the_saved_position_follows_playback() {
        let mut snapshot = PlayerSnapshot::idle();
        assert_eq!(follow(42_000, &snapshot, true), 42_000, "restored, resting");
        assert_eq!(follow(42_000, &snapshot, false), 0, "stopped");
        snapshot.state = PlaybackState::Loading;
        snapshot.position_ms = 0;
        assert_eq!(follow(42_000, &snapshot, false), 42_000, "starting");
        snapshot.state = PlaybackState::Playing;
        snapshot.position_ms = 43_500;
        assert_eq!(follow(42_000, &snapshot, false), 43_500);
        snapshot.state = PlaybackState::Ended;
        assert_eq!(follow(43_500, &snapshot, false), 0);
    }
}
