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

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::{Command, Error, PlayerHandle, PlayerSnapshot, QueueSnapshot, Result, SinkInfo};

/// Which player a connection drives (canon-973d). Each player has its own actor: its own queue,
/// playback and snapshot, kept by the server across disconnects and restarts. A player names
/// itself; until players authenticate (canon-65d2) the name is all there is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlayerId(String);

impl PlayerId {
    /// The player a connection drives when it names none: the server's own.
    pub const DEFAULT: &'static str = "default";

    /// A player id: 1 to 64 of ASCII letters, digits, `.`, `_` and `-`, so it is safe in a URL, a
    /// log line and a database key alike.
    ///
    /// # Errors
    /// [`Error::Unsupported`] naming what is wrong with it.
    pub fn parse(id: &str) -> Result<Self> {
        let fits = (1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if fits {
            Ok(Self(id.to_owned()))
        } else {
            Err(Error::Unsupported(format!(
                "{id:?} is not a player id: use 1 to 64 letters, digits, '.', '_' or '-'"
            )))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for PlayerId {
    fn default() -> Self {
        Self(Self::DEFAULT.to_owned())
    }
}

impl std::fmt::Display for PlayerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PlayerId {
    type Error = Error;
    fn try_from(id: String) -> Result<Self> {
        Self::parse(&id)
    }
}

impl From<PlayerId> for String {
    fn from(id: PlayerId) -> Self {
        id.0
    }
}

/// Every player the server keeps (canon-973d): what a connection asks for its player's control
/// plane through. A player not seen before gets an actor of its own, holding the queue it last had
/// if the server kept one.
#[async_trait]
pub trait Players: Send + Sync {
    /// Player `id`'s control plane, made on first asking.
    ///
    /// # Errors
    /// The server won't keep such a player.
    async fn player(&self, id: &PlayerId) -> Result<Arc<dyn ControlPlane>>;
}

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
