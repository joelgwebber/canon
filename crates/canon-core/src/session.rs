//! Service login + identity: the source-agnostic auth vocabulary.
//!
//! Canon federates several music services (Tidal first; Spotify's feasibility is yak
//! canon-1175), and every one of them logs in through the OAuth 2.0
//! device-authorization flow: show the user a short code + URL, poll until they
//! approve, then hold a rotating token pair. Rather than let each service crate grow
//! its own login surface — and force the control API to special-case each — that flow
//! is captured once here as [`ServiceSession`].
//!
//! Keeping it in `canon-core` is what lets [`crate`]'s dependents stay decoupled: the
//! control API (`canon-api`) drives login through `Arc<dyn ServiceSession>` and never
//! names Tidal, and a second service slots in behind the same trait. The service crate
//! (`canon-tidal`) owns the wire details, token persistence, and refresh; the API owns
//! none of it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::{Result, Service};

/// The device-authorization details the user acts on to log in.
///
/// The client shows `user_code` and sends the user to `verification_uri` (or, when the
/// service supplies it, the pre-filled `verification_uri_complete`). `interval` is the
/// minimum seconds between polls; `expires_in` is how long the code stays valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: u64,
    pub interval: u64,
}

/// Where a device-code login stands on the latest poll.
///
/// `SlowDown` is distinct from `Pending` so the caller can widen its poll interval as
/// the OAuth spec asks, rather than hammering the token endpoint into a hard failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginStatus {
    /// The user has not approved yet — poll again after the interval.
    Pending,
    /// The service asked us to poll less often — widen the interval, then poll again.
    SlowDown,
    /// The user approved; the session now holds tokens and is authenticated.
    Authorized,
}

/// The minimal "who am I" an authenticated call returns.
///
/// Its real job is to be *proof the access token works end to end*: a session can only
/// fill this in by making a call the service authorises. `user_id` is a string because
/// services disagree on its shape (Tidal's is numeric, others aren't).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub service: Service,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Service-specific extras worth surfacing (e.g. Tidal's `countryCode`). Kept as a
    /// free map so a service can report what it has without widening this type.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub attributes: std::collections::BTreeMap<String, String>,
}

/// A logged-in (or logging-in) connection to one music service.
///
/// Object-safe via `async_trait` so the daemon can hold `Arc<dyn ServiceSession>` per
/// service and the control API can drive any of them uniformly. Login is stateful —
/// [`begin_login`](ServiceSession::begin_login) stashes the in-flight authorization
/// that [`poll_login`](ServiceSession::poll_login) advances — so implementations use
/// interior mutability and every method takes `&self`.
#[async_trait]
pub trait ServiceSession: Send + Sync {
    /// Which service this session speaks to.
    fn service(&self) -> Service;

    /// Whether a usable token pair is currently held (no network I/O).
    fn is_authenticated(&self) -> bool;

    /// Begin device-code login: obtain a fresh code and stash the in-flight
    /// authorization for [`poll_login`](ServiceSession::poll_login) to advance.
    async fn begin_login(&self) -> Result<DeviceCode>;

    /// Poll the outstanding login once. On [`LoginStatus::Authorized`] the session has
    /// captured and persisted its tokens. Errors if no login is in flight.
    async fn poll_login(&self) -> Result<LoginStatus>;

    /// Make an authenticated call and report the account it belongs to — the
    /// end-to-end proof the held token is valid. Refreshes the token first if needed.
    async fn account(&self) -> Result<Account>;
}
