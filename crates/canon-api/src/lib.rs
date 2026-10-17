//! canon-api — the remote surface, thin over the core.
//!
//! Both faces translate to the one [`canon_core::Command`] vocabulary and stream the
//! one [`canon_core::PlayerSnapshot`], so playback logic never leaks into a client.
//!
//! Responsibilities (see yak canon-1190 and its children):
//! * **Control plane** (canon-b46b): a WebSocket+JSON API — commands in,
//!   snapshot-then-updates out, reconciled by `seq`. Implemented in [`server`] over the
//!   wire schema in `canon-protocol`, which clients link without this crate. Also carries
//!   the service-session (auth) verbs so a client can log a source in and make the first
//!   authenticated call.
//! * **Snapshot stream** (canon-9487): every connection gets an immediate full snapshot
//!   then one on each change; clients reconcile by `seq` and interpolate with `rate`.
//! * **MCP tools** (canon-c67f): one tool per verb over the same command bus — not yet
//!   built; it will reuse `canon-protocol`'s vocabulary.
//! * **The queue** is server state, owned by the player actor: its position rides on every
//!   snapshot, and its contents come from the `queue` op.

pub mod auth;
pub mod server;

pub use auth::{Access, StreamSigner, TokenStore};
pub use server::{AppState, router, serve};
