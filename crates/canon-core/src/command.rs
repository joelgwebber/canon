//! The command/event vocabulary of the player.
//!
//! Both the WebSocket+JSON control plane and the MCP tool layer translate into these
//! [`Command`]s — there is no second place playback logic can live (contrast tideway,
//! whose queue lived in the browser). Everything that can change playback reality is a
//! message to the one state actor, and every change comes back out as an [`Event`].

use std::time::Duration;

use crate::{PlayerSnapshot, SinkId, TrackRef};

/// A command into the player state actor.
#[derive(Debug, Clone)]
pub enum Command {
    /// Replace the current track and begin loading it.
    Load(TrackRef),
    Play,
    Pause,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    SetMuted(bool),
    /// Route audio to a different output. "Which sink" is state, not a side effect.
    SelectSink(SinkId),
}

/// An event out of the player.
///
/// For now every change is a full seq-stamped snapshot. Discrete side-events (e.g. a
/// cross-device pause arriving on Tidal's realtime bus, yak canon-333e) can be added
/// as variants without disturbing the reconcile-by-seq contract.
#[derive(Debug, Clone)]
pub enum Event {
    Player(PlayerSnapshot),
}
