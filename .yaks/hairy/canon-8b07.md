---
id: canon-8b07
title: 'Spotify Web API crate: OAuth PKCE and a Catalog (search, albums, artists, saved library, own playlists)'
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:28:11Z'
parent: canon-b3a5
labels:
- spotify
---

Step 4. New canon-spotify crate against the Web API for a personal dev-mode app (Feb 2026 rules: search max 10 per request, single-item GETs, own/collab playlist contents only, external_ids restored Mar 2026). Implements canon_core::Catalog (radio/similar/mixes -> Unsupported) incl. favorites (saved tracks/albums/artists) and playlists for import, ISRC and UPC carried. Auth: authorization code + PKCE with a loopback redirect. Live use needs Joel's developer app client id.
