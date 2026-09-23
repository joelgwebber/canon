---
id: canon-6dc7
title: 'Playlists: canon-owned, ordered, playable'
type: task
priority: 2
created: '2026-09-23T21:57:32Z'
updated: '2026-09-23T21:57:32Z'
parent: canon-4185
labels:
- library
- api
---

Playlists live in canon (never created upstream; export is canon-65f7). Schema v2: playlists(id, name, created_at, updated_at), playlist_tracks(playlist, position, track). Ops: playlists, playlist {id}, playlist_create {name, items?}, playlist_rename, playlist_delete, playlist_add {id, items, at?}, playlist_remove {id, index}, playlist_move; a playlist is an ItemRef that expands to its tracks, so queue_add plays it. canon control commands to match.
