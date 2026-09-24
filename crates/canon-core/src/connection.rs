//! Connections: the ways canon is signed in to a service, and what each one grants
//! (docs/connections.md, yak canon-c739).
//!
//! Services grant different things depending on *how* you log in: a Tidal device-code token
//! browses but can't stream, while the Android client's PKCE token streams hi-res; Spotify's Web
//! API is library-only and its audio needs a different client altogether. So what canon may do is
//! a property of a **connection** (one login method on one service), not of the service. A
//! [`Connector`] per service describes its login methods as data, runs their flows, and hands out
//! a [`Source`] or [`Catalog`] only from a connection that grants it; [`crate::Sources`] routes by
//! capability and says why when nothing qualifies ([`crate::Error::NotEntitled`]).
//!
//! One account per service per instance: a connection is named by its method (`tidal.pkce`), and
//! a connector holds at most one connection per method.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Account, Catalog, DeviceCode, LoginStatus, Quality, Result, Service, Source};

/// One thing canon can do through a connection. Routing asks for these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Search, album and artist listings, lookups.
    Catalog,
    /// The user's favorites and playlists.
    LibraryRead,
    /// Saving to the service, writing playlists there.
    LibraryWrite,
    /// Radio, similar artists, personal mixes.
    Recommendations,
    /// Full-length audio.
    Stream,
}

impl Capability {
    /// What the capability lets canon do, for messages: "Tidal can't {stream}".
    #[must_use]
    pub fn doing(self) -> &'static str {
        match self {
            Capability::Catalog => "browse",
            Capability::LibraryRead => "read your library",
            Capability::LibraryWrite => "change your library",
            Capability::Recommendations => "recommend",
            Capability::Stream => "stream",
        }
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.doing())
    }
}

/// Everything a connection grants.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub catalog: bool,
    pub library_read: bool,
    pub library_write: bool,
    pub recommendations: bool,
    /// Full-length audio, up to this quality. `None`: no audio at all.
    pub stream: Option<Quality>,
}

impl Capabilities {
    /// Whether `capability` is granted.
    #[must_use]
    pub fn has(&self, capability: Capability) -> bool {
        match capability {
            Capability::Catalog => self.catalog,
            Capability::LibraryRead => self.library_read,
            Capability::LibraryWrite => self.library_write,
            Capability::Recommendations => self.recommendations,
            Capability::Stream => self.stream.is_some(),
        }
    }
}

/// How a login method is carried out, so a client knows what to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    /// Show a short code and a URL; the user approves elsewhere; poll until done.
    DeviceCode,
    /// Open a URL in a browser; the user logs in and pastes back the URL they land on.
    Browser,
}

/// One way to sign in to a service, and what it grants, known before anyone signs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Method {
    /// `tidal.pkce`: also the id of the connection it makes.
    pub id: String,
    pub service: Service,
    pub label: String,
    pub flow: FlowKind,
    /// What a connection made this way grants.
    pub grants: Capabilities,
    /// Anything worth knowing before choosing it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// A login in progress: what the user has to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "flow", rename_all = "snake_case")]
pub enum LoginFlow {
    /// Show `code` to the user, then complete (poll) until authorized.
    DeviceCode { code: DeviceCode },
    /// Send the user to `url`; complete with the URL they land on.
    Browser { url: String },
}

/// Whether a connection is usable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Health {
    /// Signed in, and the service answered as that account.
    Ok,
    /// No credentials: sign in with this method.
    NeedsLogin,
    /// Signed in, but the service refused something the method should grant (a login Tidal
    /// stopped letting stream). Routing no longer offers it for that; `why` is what was said.
    Degraded { why: String },
    /// Credentials are held but the service refuses or can't be reached.
    Failing { why: String },
}

/// A connection as a client sees it: one login method on one service, and its state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionInfo {
    /// The method it was made with (`tidal.pkce`).
    pub id: String,
    pub service: Service,
    pub label: String,
    /// What its method grants, on paper.
    pub grants: Capabilities,
    /// What use or a probe has shown it really grants, once known. Services move their gates
    /// (Tidal has, three times in a year), so this can fall short of `grants`. Its `stream` is
    /// the best quality seen, a floor: a probe track without a hi-res master shows lossless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<Capabilities>,
    pub health: Health,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Account>,
}

/// One service's ways in: its login methods, the connections made with them, and what those
/// connections give canon to work with.
#[async_trait]
pub trait Connector: Send + Sync {
    fn service(&self) -> Service;

    /// Every way to sign in, whether or not one is in use.
    fn methods(&self) -> Vec<Method>;

    /// Each method's connection state. Checks every held login with the service, so it makes
    /// network calls.
    async fn connections(&self) -> Vec<ConnectionInfo>;

    /// Start signing in with `method`.
    ///
    /// # Errors
    /// No such method, or the service refused to start.
    async fn begin(&self, method: &str) -> Result<LoginFlow>;

    /// Advance a login begun with [`Connector::begin`]: poll a device code (`input` is `None`),
    /// or finish a browser login with the URL the user landed on.
    ///
    /// # Errors
    /// No login in flight for `method`, or the service refused it.
    async fn complete(&self, method: &str, input: Option<String>) -> Result<LoginStatus>;

    /// Forget `method`'s login.
    ///
    /// # Errors
    /// The credentials couldn't be removed.
    async fn disconnect(&self, method: &str) -> Result<()>;

    /// Whether a held login grants `capability`. Local and cheap: the answer comes from held
    /// logins, not from asking the service.
    fn grants(&self, capability: Capability) -> bool;

    /// A [`Source`] from the connection best placed for `need`: opening audio needs
    /// [`Capability::Stream`] (the connection that streams best), describing a track needs only
    /// [`Capability::Catalog`]. `None` if no connection grants it.
    fn source(&self, need: Capability) -> Option<Arc<dyn Source>>;

    /// Browsing, from a connection that grants it. `None` if none can.
    fn catalog(&self) -> Option<Arc<dyn Catalog>>;

    /// What to do to get `capability`, for an error message: "sign in with the streaming login".
    fn hint(&self, capability: Capability) -> String;
}
