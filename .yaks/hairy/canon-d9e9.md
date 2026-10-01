---
id: canon-d9e9
title: Playlists in search results, and open a playlist by URL/id
type: task
priority: 3
created: '2026-10-01T01:22:03Z'
updated: '2026-10-01T01:22:03Z'
parent: canon-65f7
depends_on:
- canon-5b2b
labels:
- library
- catalog
---

SearchResults currently holds only tracks, albums and artists. Add playlists (and mixes where a service supports it) to search, and support opening a remote playlist directly by its URL/id (there's no open-by-id today). Spotify's algorithmic playlists have been unreachable via the API since late 2024, so this is about ordinary public/followed/owned playlists.
