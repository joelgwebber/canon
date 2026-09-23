---
id: canon-4fb2
title: 'Import from Tidal: favorites into saved, playlists into canon playlists'
type: task
priority: 2
created: '2026-09-23T22:06:34Z'
updated: '2026-09-23T22:11:02Z'
parent: canon-65f7
labels:
- library
- tidal
- api
verify: cargo test -p canon-library an_import_is_idempotent && cargo test -p canon-tidal catalog
---

So the library starts as the user's, not empty. Catalog::favorites (tracks/albums/artists with when they were added) and Catalog::playlists (the account's playlists with their tracks). Library::import saves favorites with their original dates and brings each service playlist in as a canon playlist bound to its service id, so importing again updates it instead of duplicating. One-way: canon never writes back (export is canon-65f7 proper). API op import; canon control import.

---
▸ 2026-09-23T22:10:57Z [Joel Webber]
Built: Catalog::favorites (tracks/albums/artists with added_ms) and Catalog::playlists (own playlists with their tracks); Tidal over /v1/users/<uid>/favorites/{tracks,albums,artists} (order=DATE desc) and /v1/users/<uid>/playlists + /v1/playlists/<uuid>/items (videos skipped); epoch_ms parser for Tidal timestamps. Library::import (one transaction): favorites ingested + save_at(original date, first save wins); playlists bound to their Tidal uuid, so a re-import renames/replaces instead of duplicating. API op import; canon control import.

---
▸ 2026-09-23T22:10:57Z [Joel Webber]
Live on the real account 2026-09-23: "imported 280 tracks, 133 albums, 146 artists, 8 playlists" (~3s), twice, with no duplicates (8 playlists: Sci-Fi 135, Primus 16, Post Rock 178, Neoclassical 31, Metallic 182, Jazz practice 5, ...). Found and fixed a paging bug on the way: Tidal pages first and then drops tracks unavailable in the country (a page of 100 comes back with 96), and the pager advanced by items received, re-reading page tails (319 "favorites" with ~48 repeats). It now advances by the window requested. The 280 favorites are 271 distinct Tidal ids (Tidal lists 9 twice) and 256 recordings: 15 were the same recording under another id, matched by ISRC (e.g. Mogwai "Ether" x3).

---
▸ 2026-09-23T22:11:02Z [Joel Webber]
verify: `cargo test -p canon-library an_import_is_idempotent && cargo test -p canon-tidal catalog` -> PASS (exit 0)
