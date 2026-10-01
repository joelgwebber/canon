---
id: canon-8ed0
title: 'Export: create a new upstream playlist from a local one'
type: task
priority: 3
created: '2026-10-01T01:21:55Z'
updated: '2026-10-01T01:57:33Z'
parent: canon-65f7
depends_on:
- canon-5b2b
labels:
- library
- api
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

export_playlist(id, service): creates a NEW playlist on the service from the local playlist's tracks (resolving each to that service's catalog), never updates or deletes an existing upstream playlist. Description stamped with something like 'Exported from canon, <date>' so repeat exports are easy to find and delete by hand. Returns a report of tracks that didn't resolve on that service. Re-export = nuke the old one upstream yourself and export again; canon does not delete upstream (an export log to support canon deleting its own copies later is deferred).

---
▸ 2026-10-01T01:57:33Z [Joel Webber]
Claimed for parallel lane canon-8ed0 (worktree wt/canon-8ed0). Scope: Library::export_playlist (lib.rs), a new write-capable seam on the service side, api/protocol.rs, control.rs. Hints: Capability::LibraryWrite already exists in canon-core/src/connection.rs and crates/canon-spotify/src/connector.rs:357 already asserts Spotify grants it (playlist-modify-private/public scopes are already requested) -- Tidal write support is unknown/unproven, treat it as optional/Unsupported for v1 if its API does not clearly support it. Mirror the existing Connector::catalog(&self) -> Option<Arc<dyn Catalog>> pattern (crates/canon-core/src/connection.rs ~L191-198) for a new write-capable accessor with a default None impl, so other connectors are not forced to implement it. Library::match_onto(sources, track, service) (lib.rs ~L895) already resolves a local track onto a target services SourceRef -- reuse it to resolve each track before creating the upstream playlist. Stamp the created playlists description noting it was exported from canon (canon-65f7 decision: canon never deletes upstream; re-export means the user deletes the old one by hand). Evidence: the verify command above. This is the most exploratory of the three parallel lanes -- investigate the real API shape (canon tidal-get, Spotify API docs) before committing to a design, and it is fine to land Spotify-only if Tidal write is not practical yet. Coordinator resolves any textual conflict in protocol.rs/control.rs against sibling lanes at merge.
