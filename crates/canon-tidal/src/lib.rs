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
//! # Spike status (yak canon-a880)
//!
//! This crate currently contains a **de-risking spike**, not the finished Source:
//! * [`http`] — the swappable [`TidalHttp`] seam and its `wreq` browser-impersonation
//!   backend ([`http::WreqHttp`]).
//! * [`auth`] — a compile-only, typed skeleton of Tidal's device-code OAuth flow.
//!
//! [`canon_core::Source`] is intentionally **not** implemented yet; the spike proves the
//! HTTP/fingerprint foundation those flows will sit on.

pub mod auth;
pub mod http;
pub mod session;
pub mod store;

pub use http::{HttpResponse, TidalHttp, WreqHttp};
pub use session::TidalSession;
pub use store::{PersistedTokens, TokenStore};
