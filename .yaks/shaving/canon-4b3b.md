---
id: canon-4b3b
title: Merge a source into a local playlist, append-only by recording identity
type: task
priority: 2
created: '2026-10-01T01:21:45Z'
updated: '2026-10-01T01:57:32Z'
parent: canon-65f7
depends_on:
- canon-5b2b
- canon-120d
labels:
- library
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

playlist_add gets a merge mode: given a source (local or remote, via the canon-5b2b facade), append only the tracks whose recording isn't already in the target playlist (match by the same recording-identity canon already uses for cross-service matching -- ISRC/merged-track aliasing, not raw track id). Removals upstream never come across; the library is the source of truth. This is how re-syncing an upstream playlist works now that import no longer touches playlists.

---
▸ 2026-10-01T01:57:32Z [Joel Webber]
Claimed for parallel lane canon-4b3b (worktree wt/canon-4b3b). Scope: a merge mode for playlist_add (lib.rs/store.rs), api/protocol.rs (likely a field on PlaylistAdd or a new op), control.rs. Hint: ingest_track already unifies by ISRC (tracks_with_isrc), so two tracks naming the same recording already resolve to the same EntityId after ingest -- "recording identity" dedup is likely just "skip ids already in the target list after ingesting the source", nothing more exotic needed. Evidence: the verify command above. Coordinator resolves any textual conflict in protocol.rs/control.rs against sibling lanes 8ed0/d9e9 at merge.
