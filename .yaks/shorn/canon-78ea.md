---
id: canon-78ea
title: Canonical entity model + persistence (sqlite)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-23T21:03:11Z'
parent: canon-4185
labels:
- library
verify: cargo test -p canon-library
---

canon-native Track/Release/Artist entities with stable canon IDs, persisted (rusqlite/sqlx) — not tideway's rebuilt-every-launch in-memory map. Each entity carries external references. The identity/join-key strategy (canon-native + ISRC vs adopting MusicBrainz MBIDs) is the open question on this yak.

---
▸ 2026-09-22T02:01:40Z [claude]
Library identity: canon-native IDs with ISRC as the join key (simple, self-contained) vs adopting MusicBrainz recording/release MBIDs as the canonical identity (richer cross-service matching + metadata, but a real external dependency and modelling cost). Which way should the entity model lean?

---
▸ 2026-09-22T02:43:39Z [Joel Webber]
I'd really like to be able to match tracks across services and local music, so let's bite the bullet and take on MBIDs.

---
▸ 2026-09-23T20:58:25Z [Joel Webber]
Design (2026-09-23), following MusicBrainz where it matters for matching:
- Track = a RECORDING (the audio), keyed across services by ISRC and recording MBID. Bindings attach here: any binding of a recording plays the same audio, so the album cut and the compilation cut of one recording are one Track with two Tidal bindings (and one play count, one "liked").
- Album = a RELEASE (one edition: its own tracklist, barcode/UPC; Tidal albums are editions). Optional release-group MBID clusters editions. Tracklist is album_tracks(album, disc, position, track), so the same recording can sit on many albums.
- Artist, with credits as ordered (entity, position, artist) rows plus the display credit string.
- One EntityId (uuid) space for all kinds; bindings are (entity, service, key) with provenance (direct/isrc/mbid/fuzzy/manual) and confidence, unique on (kind, service, key) so a service id maps to exactly one entity.
- The store is everything canon knows, not just the user library: browsing and the queue mint entities too (through the library, never at the edge). Library membership ("saved") is a separate table; playlists come later with canon-65f7.
- rusqlite (bundled sqlite, no system dep), one connection behind a mutex on the blocking pool, schema migrations keyed on PRAGMA user_version. File: <state_dir>/library.sqlite.

---
▸ 2026-09-23T21:03:11Z [Joel Webber]
Built canon-library: model (Track=recording w/ isrcs+mbid, Album=release w/ barcode, mbid, group_mbid, Artist; ordered credits; AlbumTrack), schema v1 via user_version migrations (refuses a newer file), Store (rusqlite bundled 0.37, WAL, foreign keys) with add/update/get, tracklists, albums_of, tracks_with_isrc, albums_with_barcode, by_mbid, kind_of, bind/unbind/bound/bindings (one entity per service key; stronger provenance kept), save/unsave/saved, track_ref (names from credits, first album, bindings most trusted first); Library facade runs closures on the blocking pool. Added Error::Library. 10 tests pass. Not wired into the daemon yet: that is canon-f7da (the library mints ids at the API edge).

---
▸ 2026-09-23T21:03:11Z [Joel Webber]
verify: `cargo test -p canon-library` -> PASS (exit 0)
