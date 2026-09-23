//! The control-plane seam the remote API drives (yak canon-a7d6).
//!
//! `canon-api` translates client frames into [`Command`]s and streams
//! [`PlayerSnapshot`]s, but it must not know *how* a command becomes sound — whether
//! there's a real audio engine and a network source behind it, or just the state actor.
//! [`ControlPlane`] is that boundary: a dispatch-a-command / subscribe-to-snapshots
//! surface the API depends on abstractly.
//!
//! Two implementors exist:
//! * [`PlayerHandle`] itself — state-only, for tests and any headless "state mirror"
//!   use (dispatch just forwards the command; nothing is decoded).
//! * the daemon's playback controller — resolves via a [`crate::Source`], drives the
//!   audio engine on the shared clock, and folds engine events back into the player.

use async_trait::async_trait;
use tokio::sync::watch;

use crate::{Command, PlayerHandle, PlayerSnapshot, QueueSnapshot, Result, SinkInfo};

/// A command sink + snapshot source: everything the remote API needs, and nothing about
/// audio or sources.
#[async_trait]
pub trait ControlPlane: Send + Sync {
    /// Execute one command. Returns quickly; slow work (resolution, decode) happens in
    /// the background and surfaces through the snapshot stream. An immediately-known
    /// rejection (e.g. an unsupported op) is an `Err`.
    async fn dispatch(&self, command: Command) -> Result<()>;

    /// Subscribe to the authoritative snapshot stream (latest value + changes).
    fn subscribe(&self) -> watch::Receiver<PlayerSnapshot>;

    /// The current authoritative snapshot.
    fn snapshot(&self) -> PlayerSnapshot;

    /// The queue's contents (its position and revision are in every snapshot).
    fn queue(&self) -> QueueSnapshot;

    /// The outputs a client may select, local first. The default lists only the local device, so
    /// a state-only control plane needs no discovery; an implementation with network sinks
    /// overrides it with the live discovery snapshot.
    fn sinks(&self) -> Vec<SinkInfo> {
        vec![SinkInfo::local()]
    }
}

/// State-only control plane: forwards commands to the actor with no audio behind them.
#[async_trait]
impl ControlPlane for PlayerHandle {
    async fn dispatch(&self, command: Command) -> Result<()> {
        self.command(command).await
    }

    fn subscribe(&self) -> watch::Receiver<PlayerSnapshot> {
        PlayerHandle::subscribe(self)
    }

    fn snapshot(&self) -> PlayerSnapshot {
        PlayerHandle::snapshot(self)
    }

    fn queue(&self) -> QueueSnapshot {
        PlayerHandle::queue(self)
    }
}
