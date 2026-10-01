---
id: canon-f917
title: Unbind imported playlists; stop import from touching playlists
type: task
priority: 2
created: '2026-10-01T01:21:29Z'
updated: '2026-10-01T01:36:43Z'
parent: canon-65f7
labels:
- library
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

Remove the playlist-binding path from Library::import (canon-4fb2): bulk import goes back to favorites only. Add a migration that drops any existing playlist bindings (kind=Playlist in the bindings table) without touching the playlists or their tracks -- an imported playlist just becomes an ordinary local one, edits and all. This is the yak that stops destroying local edits on re-import; do it before anything else in this design.

---
▸ 2026-10-01T01:36:24Z [Joel Webber]
Removed the playlist loop from Library::import (favorites only now); added schema migration 7 (DELETE FROM bindings WHERE kind = playlist) so a playlist bound by the old import becomes an ordinary local one, untouched otherwise. Updated an_import_is_idempotent (ImportReport.playlists now always 0, no playlist assertions) and added upgrading_unbinds_a_playlist_imported_under_the_old_scheme to store.rs. Fixed README.md and docs/arch.md passages that described import touching playlists.

---
▸ 2026-10-01T01:36:39Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
