---
id: canon-b989
title: 'Catalog browsing: search, album and artist listings from a source'
type: task
priority: 2
created: '2026-09-23T20:04:16Z'
updated: '2026-09-23T21:57:14Z'
parent: canon-4185
depends_on:
- canon-ba9d
labels:
- library
- tidal
- api
verify: cargo test -p canon-library && cargo test -p canon-tidal catalog
---

Found needing Pink Floyd track ids (2026-09-23): canon can play a track id but cannot find one. It took a raw call to api.tidal.com/v1/albums/<id>/tracks with the stored token.

Scope: a browse/catalog seam next to Source (a source that can play is not always one that can search), implemented for Tidal: search (tracks/albums/artists), album tracks, and artist albums, all through the wreq session and token refresh. Surfaced as API ops and in canon control (`search <text>`, `album <id>` listing ids ready to enqueue; maybe `enqueue album:<id>`).

Design note for the library epic: results are upstream *references*, not library entities. What a listing returns should convert into TrackRef candidates that the library resolves and binds (canon-880e / canon-f7da), so that browsing never mints identity at the edge.

---
▸ 2026-09-23T21:57:13Z [Joel Webber]
Built: canon_core::Catalog (search, album, artist, radio(Seed), similar_artists) + SearchResults/AlbumListing/ArtistListing, registered via Sources::with_catalog. Tidal: catalog.rs over /v1/search, /v1/albums/<id>(+/tracks paged), /v1/artists/<id>(+/albums, EPSANDSINGLES, /toptracks), /radio, /similar; one api_get helper; titles get their version unless already in them. Library: search/album/artist ingest and return views (TrackView/AlbumView/ArtistView with canon ids, saved flag, bindings); album opening ingests the whole listing (replacing a pieced-together tracklist, merging credits/date/barcode); tracks_for(album) fetches the listing first. API ops search/album/artist; canon control search/album/artist with numbered #n listings, playfrom #n.

---
▸ 2026-09-23T21:57:13Z [Joel Webber]
Live against Tidal 2026-09-23 (local output): `search pink floyd dark side of the moon` -> 10 tracks / 10 albums / 1 artist numbered; `album #11` -> The Dark Side of the Moon, all 10 tracks with durations; `playfrom #9` -> queue of 10 with > 9. Brain Damage, then status "playing 0:22/3:50 Brain Damage [9/10]"; `artist 9706` -> 10 top tracks + 41 releases newest first. Found and fixed: titles that already contain their version got it twice ("(2025 Mix) (2025 Mix)").

---
▸ 2026-09-23T21:57:14Z [Joel Webber]
verify: `cargo test -p canon-library && cargo test -p canon-tidal catalog` -> PASS (exit 0)
