---
id: canon-6dc7
title: 'Playlists: canon-owned, ordered, playable'
type: task
priority: 2
created: '2026-09-23T21:57:32Z'
updated: '2026-09-23T22:06:05Z'
parent: canon-4185
labels:
- library
- api
verify: cargo test -p canon-library playlist
---

Playlists live in canon (never created upstream; export is canon-65f7). Schema v2: playlists(id, name, created_at, updated_at), playlist_tracks(playlist, position, track). Ops: playlists, playlist {id}, playlist_create {name, items?}, playlist_rename, playlist_delete, playlist_add {id, items, at?}, playlist_remove {id, index}, playlist_move; a playlist is an ItemRef that expands to its tracks, so queue_add plays it. canon control commands to match.

---
▸ 2026-09-23T22:06:02Z [Joel Webber]
Built: EntityKind::Playlist in the one id space; schema v2 (playlists, playlist_tracks with dense positions, cascade on delete, tracks kept); Store create/rename/delete/edit_playlist/playlist_view/detail; kind_of and expand know playlists (so queue_add plays them); library kind=playlist lists all playlists (they are always the user own; save refuses them). Library async ops take ItemRefs (albums expand). API ops playlist, playlist_create/rename/delete/add/remove/move. canon control: pl [list], pl new|fromqueue <name>, pl use #n, pl show|play|add|rm|mv|rename|delete.

---
▸ 2026-09-23T22:06:03Z [Joel Webber]
Live 2026-09-23 (migrated the real library.sqlite v1 -> v2): pl new Night drive; pl add #1 (Army of Me) album:55391786 -> 11 tracks; pl mv 11 1 (Eclipse to the top); pl rm 2 (Army of Me out); pl rename; pl play -> queue of 10 with Eclipse current; pl fromqueue -> "From the queue (10 tracks)"; pl lists both; both deleted -> playlists count 0, user_version 2.

---
▸ 2026-09-23T22:06:04Z [Joel Webber]
verify: `cargo test -p canon-library playlist` -> PASS (exit 0)
