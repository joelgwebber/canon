//! canon-tidal — the Tidal implementation of [`canon_core::Source`].
//!
//! Responsibilities (see yak canon-94cc and its children):
//! * **Auth + token lifecycle** (canon-8bab): device-code and PKCE flows; capture the
//!   *rotating* refresh token on every refresh; single-flight refresh.
//! * **Stream resolution** (canon-4c55): decode the base64 DASH/MPD manifest into an
//!   init + media fMP4 segment list; reject encrypted manifests; short-TTL cache.
//! * **Segment reader** (canon-e99d): present the segment list as a plain seekable
//!   [`canon_core::MediaInput`], transparently re-resolving expired (403) segment URLs
//!   and resuming at the current index — the fix for tideway tide-1100/bd9e.
//! * **Realtime bus + play reporting** (canon-333e): the Pushkin websocket for
//!   cross-device pause, and event-batch reporting so canon shows in Recently Played.
//! * All HTTP flows through the TLS-fingerprint-impersonation layer (canon-a880): a
//!   pure-Rust `wreq` spike first, C-FFI `curl-impersonate` as the accepted fallback.
//!
//! The whole surface is reverse-engineered and unversioned; the accepted posture
//! (yak canon-94cc) is personal-use, degrade-honestly, diagnostic-first.
//!
//! # Status
//!
//! Auth is **live** end to end (yak canon-8bab): [`TidalSession`] implements
//! [`canon_core::ServiceSession`] over the [`auth`] flow and the [`http`]
//! browser-impersonation client, drives the device-code login against the real Tidal
//! endpoints, refreshes tokens single-flight, persists them via [`store`], and makes the
//! first authenticated call (`GET /v1/sessions`). Verified against the live service by
//! `canon login tidal`, which returns a real device code (the `deviceCode` camelCase
//! shape was confirmed against the live endpoint).
//!
//! Playback is live too: [`TidalSource`] is the [`canon_core::Source`] the daemon
//! registers, opening a track at any position as a lazily fetched segment stream
//! (canon-4c55, canon-e99d). Still to come: the realtime bus (canon-333e).

pub mod auth;
pub mod http;
mod segment;
pub mod session;
mod source;
pub mod store;
pub mod stream;

pub use http::{HttpResponse, TidalHttp, WreqHttp};
pub use session::TidalSession;
pub use source::TidalSource;
pub use store::{PersistedTokens, TokenStore};
pub use stream::ResolvedTidalStream;
