//! The command/event vocabulary of the player.
//!
//! Both the WebSocket+JSON control plane and the MCP tool layer translate into these
//! [`Command`]s — there is no second place playback logic can live (contrast tideway,
//! whose queue lived in the browser). Everything that can change playback reality is a
//! message to the one state actor, and every change comes back out as a new snapshot.

use std::time::Duration;

use crate::{Repeat, SinkId, TrackRef};

/// A command into the player state actor: user intent.
///
/// The actor owns the queue, so the queue verbs are transitions like any other. A command the
/// actor cannot apply (no next track, nothing to seek) is rejected with an error; one that changes
/// nothing (pausing while paused) is accepted and is not a transition.
#[derive(Debug, Clone)]
pub enum Command {
    /// Replace the queue with this single track and begin loading it.
    Load(TrackRef),
    /// Append a track to the queue (starting playback if idle).
    Enqueue(TrackRef),
    /// Append tracks to the queue, starting the first of them if nothing is in play.
    EnqueueMany(Vec<TrackRef>),
    /// Insert tracks right after the current entry.
    PlayNext(Vec<TrackRef>),
    /// Replace the queue with `tracks` and start the entry at `start`: an album from its third
    /// track, a search result list from the one picked.
    Replace {
        tracks: Vec<TrackRef>,
        start: usize,
    },
    /// Start the entry at `index`.
    Jump(usize),
    /// Remove the entry at `index`. Removing the one playing moves on to the entry after it.
    Remove(usize),
    /// Move the entry at `from` so it sits at `to`.
    Move {
        from: usize,
        to: usize,
    },
    /// Shuffle the entries after the current one.
    Shuffle,
    /// What happens at the end of a track, and at the end of the queue.
    SetRepeat(Repeat),
    /// Skip to the next queued track.
    Next,
    /// Skip to the previous queued track.
    Previous,
    /// Clear the queue and stop.
    Clear,
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    SetMuted(bool),
    /// Route audio to a different output. "Which sink" is state, not a side effect.
    SelectSink(SinkId),
}
