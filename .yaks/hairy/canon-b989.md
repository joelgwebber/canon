---
id: canon-b989
title: 'Catalog browsing: search, album and artist listings from a source'
type: task
priority: 2
created: '2026-09-23T20:04:16Z'
updated: '2026-09-23T20:04:21Z'
parent: canon-4185
depends_on:
- canon-ba9d
labels:
- library
- tidal
- api
---

Found needing Pink Floyd track ids (2026-09-23): canon can play a track id but cannot find one. It took a raw call to api.tidal.com/v1/albums/<id>/tracks with the stored token.

Scope: a browse/catalog seam next to Source (a source that can play is not always one that can search), implemented for Tidal: search (tracks/albums/artists), album tracks, and artist albums, all through the wreq session and token refresh. Surfaced as API ops and in canon control (`search <text>`, `album <id>` listing ids ready to enqueue; maybe `enqueue album:<id>`).

Design note for the library epic: results are upstream *references*, not library entities. What a listing returns should convert into TrackRef candidates that the library resolves and binds (canon-880e / canon-f7da), so that browsing never mints identity at the edge.
