---
id: canon-120d
title: 'Playlist history: version on every edit, list, restore'
type: task
priority: 2
created: '2026-10-01T01:21:34Z'
updated: '2026-10-01T01:38:28Z'
parent: canon-65f7
depends_on:
- canon-f917
labels:
- library
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

A version is recorded whenever a local playlist's track list changes (add/remove/move/merge/create). List a playlist's versions and restore one. Comes before merge so a bad merge is always undoable. Keep it simple: snapshot the ordered track list per version, no diffing yet (diff is deferred).

---
▸ 2026-10-01T01:38:28Z [Joel Webber]
Claimed for parallel lane canon-120d (worktree wt/canon-120d). Scope: schema.rs (new migration + playlist_versions table), store.rs (hook create_playlist/edit_playlist to record a version; new list/restore version methods), lib.rs (new Library::playlist_versions/restore_playlist_version near playlist_move ~L591-606), canon-api protocol.rs + server.rs (new ops), canon-daemon control.rs (pl versions/pl restore, see playlist_command). Evidence: the verify command above, judged by doctor --strict at shear. Coordinator resolves any textual conflict in schema.rs/protocol.rs against sibling lanes 5b2b/ae6e at merge -- that is expected, not a sign of a design clash.
