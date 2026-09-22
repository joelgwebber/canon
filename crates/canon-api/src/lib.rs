//! canon-api — the remote surface, thin over the core.
//!
//! Both faces translate to the one [`canon_core::Command`] vocabulary and stream the
//! one [`canon_core::PlayerSnapshot`], so playback logic never leaks into a client.
//!
//! Responsibilities (see yak canon-1190 and its children):
//! * **Control plane** (canon-b46b): a WebSocket+JSON API (the chosen transport) —
//!   commands in, snapshot-then-deltas out, reconciled by `seq`. Owns the queue
//!   server-side so all clients and the OS integrations share one truth.
//! * **MCP tools** (canon-c67f): one tool per verb (play, pause, enqueue, search,
//!   resolve_track, add_to_playlist, import_playlist, pick_device) over the same
//!   command bus, so an agent can drive playback and curate the library.

use canon_core as _; // implemented against next.
