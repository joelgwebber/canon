---
id: canon-4b3b
title: Merge a source into a local playlist, append-only by recording identity
type: task
priority: 2
created: '2026-10-01T01:21:45Z'
updated: '2026-10-01T02:10:16Z'
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

---
▸ 2026-10-01T02:09:23Z [Joel Webber]
Built: Library::playlist_merge (lib.rs), a shared add_to_playlist(.., merge) behind playlist_add/playlist_merge; `merge: bool` (serde default false) on the PlaylistAdd op; server.rs routes on it; `pl merge <item>...` in control.rs. Dedup is a HashSet of the EntityIds the list already holds, extended as the batch is spliced -- so it also collapses a source that names one recording twice. No store.rs change: it goes through edit_playlist, so canon-120d records a version, and record_version already skips a list that did not change.

Unit evidence (cargo test -p canon-library --lib merging):
  running 2 tests
  test tests::merging_the_same_recording_from_another_service_adds_nothing ... ok
  test tests::merging_a_source_appends_only_the_recordings_the_playlist_lacks ... ok
  test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 59 filtered out

Wire evidence (daemon on 127.0.0.1:7399, empty --state-dir, so signed out):
  playlist: MergeProbe (0 tracks)
  error: tidal can't browse: sign in to Tidal (tidal.pkce, or tidal.device for browsing only)   <- pl add album:55391786
  error: tidal can't browse: sign in to Tidal (tidal.pkce, or tidal.device for browsing only)   <- pl merge album:55391786
`pl merge` reaches the library and fails exactly as `pl add` does, so the op field and the dispatch are wired; `pl merge` with no items prints its usage.

NOT verified end to end against real data: `canon serve` on the shared state dir will not start -- see the note on canon-65f7.

---
▸ 2026-10-01T02:10:08Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
