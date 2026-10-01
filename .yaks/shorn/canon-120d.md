---
id: canon-120d
title: 'Playlist history: version on every edit, list, restore'
type: task
priority: 2
created: '2026-10-01T01:21:34Z'
updated: '2026-10-01T01:51:16Z'
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

---
▸ 2026-10-01T01:50:55Z [Joel Webber]
Built it. Schema migration 8 adds playlist_versions (playlist, version, created_at) plus playlist_version_tracks (playlist, version, position, track), both cascading from playlists -- a header row per version so a version that empties a playlist is still a version. Store::write_playlist is the single hook: create_playlist and edit_playlist both go through it, so 'a version on every edit' is true by construction rather than by remembering to call something. record_version skips when the list is unchanged (pl mv 1 1, restoring what is already current). New Store::playlist_versions/restore_playlist_version; restore routes back through edit_playlist, so it is recorded like any other edit. merge_tracks now re-points playlist_version_tracks too -- without it, folding a track away hits the FK and the merge fails. New view::PlaylistVersion; Library::playlist_versions/restore_playlist_version; protocol ops playlist_versions/playlist_restore + ReplyData::PlaylistVersions; control 'pl versions' / 'pl restore <n>' with a coarse 'Nd/Nh/Nm/Ns ago' helper.

---
▸ 2026-10-01T01:51:04Z [Joel Webber]
Live smoke over the control plane (own port 7399, real state dir, daemon stopped by that port afterwards). printf of pl new / pl add 33348478 520285418 / pl rm 2 / pl versions / pl restore / pl delete:

  no playlists
  playlist: canon-120d smoke (0 tracks)
    v3    1 track  0s ago  (current)
    v2    2 tracks  0s ago
    v1    0 tracks  0s ago
    v4    0 tracks  0s ago  (current)
    v3    1 track  0s ago
    v2    2 tracks  0s ago
    v1    0 tracks  0s ago
  no playlists

An earlier pass also showed 'pl restore 2' putting both tracks back and 'pl restore 99' answering 'error: not found: version 99 of playlist d615dace-...'. The playlist is gone at the end, so nothing was left in the real library but the two ingested test tracks.

---
▸ 2026-10-01T01:51:12Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
