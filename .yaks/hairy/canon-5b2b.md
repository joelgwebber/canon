---
id: canon-5b2b
title: 'Track sources: service playlists/mixes as ItemRefs, Catalog::playlist(id), list a service''s playlists and mixes'
type: task
priority: 2
created: '2026-10-01T01:21:40Z'
updated: '2026-10-01T01:21:40Z'
parent: canon-65f7
depends_on:
- canon-f917
labels:
- library
- catalog
---

Make a remote playlist or mix a first-class, read-only ordered-tracks source, same shape as a local playlist from the client's perspective: ItemRef::Service{kind:Playlist} and ItemRef::Mix already exist in the queue path -- extend playlist ops (create_playlist/playlist_add) to accept any ItemRef as a source, not just local entities. Add Catalog::playlist(id) (today there's only bulk playlists()). Add a way to list a service's own playlists and mixes for browsing (mixes already have Catalog::mixes/mix; playlists need a lightweight list-without-tracks variant alongside the existing bulk playlists()).
