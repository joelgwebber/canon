//! Autoplay (yak canon-3842): when the last track in the queue starts, top the queue up with
//! tracks like it, so playback carries on instead of stopping.
//!
//! It runs beside the player rather than in it: the actor makes no network calls, and deciding
//! *what* to add needs the library and a catalog. Topping up when the last entry *starts* (not
//! when it ends) leaves time for the next track to be prepared, so the join stays gapless.

use std::sync::Arc;

use canon_core::{
    Command, EntityId, PlaybackState, PlayerHandle, PlayerSnapshot, Repeat, SettingsStore, Sources,
};
use canon_library::{ItemRef, Library};

/// How many tracks one top-up adds.
const TOP_UP: usize = 25;

pub fn spawn(
    player: PlayerHandle,
    library: Library,
    sources: Sources,
    settings: Arc<dyn SettingsStore>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut snapshots = player.subscribe();
        // The last (track, queue length) a top-up was tried for. Snapshots arrive several times a
        // second; one attempt per queue end, even a failed one, keeps a dead catalog from being
        // asked over and over.
        let mut tried: Option<(EntityId, usize)> = None;
        while snapshots.changed().await.is_ok() {
            let snapshot = snapshots.borrow_and_update().clone();
            let Some(seed) = due(&snapshot) else {
                continue;
            };
            if !settings.get().queue.autoplay || tried == Some((seed, snapshot.queue.len)) {
                continue;
            }
            tried = Some((seed, snapshot.queue.len));
            if let Err(e) = top_up(&player, &library, &sources, seed).await {
                tracing::warn!("autoplay: couldn't find more to play: {e}");
            }
        }
    })
}

/// The track to seed a top-up with, if the queue is about to run out: its last entry is the one
/// playing (or starting), and the queue doesn't repeat.
fn due(snapshot: &PlayerSnapshot) -> Option<EntityId> {
    let track = snapshot.track.as_ref()?;
    let queue = &snapshot.queue;
    let last = queue.len > 0 && queue.index + 1 == queue.len;
    let playing = matches!(
        snapshot.state,
        PlaybackState::Playing | PlaybackState::Loading
    );
    (last && playing && queue.repeat == Repeat::Off).then_some(track.id)
}

async fn top_up(
    player: &PlayerHandle,
    library: &Library,
    sources: &Sources,
    seed: EntityId,
) -> canon_core::Result<()> {
    let radio = library
        .radio(sources, &ItemRef::Entity { entity: seed })
        .await?;
    // Nothing already queued (the seed included) comes round again straight away.
    let queued: Vec<EntityId> = player.queue().tracks.iter().map(|t| t.id).collect();
    let fresh: Vec<EntityId> = radio
        .into_iter()
        .filter(|id| !queued.contains(id))
        .take(TOP_UP)
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let tracks = library
        .playable(sources, library.track_refs(fresh).await?)
        .await?;
    tracing::info!("autoplay: adding {} tracks", tracks.len());
    player.command(Command::EnqueueMany(tracks)).await
}

#[cfg(test)]
mod tests {
    use canon_core::{QueueView, TrackMeta, TrackRef};

    use super::*;

    fn snapshot(state: PlaybackState, index: usize, len: usize, repeat: Repeat) -> PlayerSnapshot {
        PlayerSnapshot {
            seq: 1,
            state,
            track: Some(TrackRef {
                id: EntityId::new(),
                meta: TrackMeta::default(),
                sources: Vec::new(),
            }),
            position_ms: 0,
            duration_ms: None,
            rate: 1.0,
            volume: 1.0,
            muted: false,
            sink: None,
            error: None,
            queue: QueueView {
                len,
                index,
                revision: 1,
                repeat,
            },
        }
    }

    #[test]
    fn only_the_last_entry_playing_without_repeat_is_due() {
        let playing_last = snapshot(PlaybackState::Playing, 2, 3, Repeat::Off);
        assert_eq!(
            due(&playing_last),
            playing_last.track.as_ref().map(|t| t.id)
        );
        assert!(due(&snapshot(PlaybackState::Loading, 0, 1, Repeat::Off)).is_some());
        assert!(due(&snapshot(PlaybackState::Playing, 1, 3, Repeat::Off)).is_none());
        assert!(due(&snapshot(PlaybackState::Paused, 2, 3, Repeat::Off)).is_none());
        assert!(due(&snapshot(PlaybackState::Playing, 2, 3, Repeat::All)).is_none());
    }
}
