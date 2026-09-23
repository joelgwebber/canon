---
id: canon-f7da
title: Library is the only minter of EntityId; SourceRef::Local is provisional
type: task
priority: 2
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T21:09:16Z'
parent: canon-4185
depends_on:
- canon-78ea
labels:
- arch
- library
verify: cargo test -p canon-library && cargo test -p canon-api
---

From canon-ba30 (I). canon-api play_track/enqueue build a TrackRef with EntityId::new() per call, so the same Tidal track enqueued twice is two entities. Once canon-78ea exists, the API must resolve (service, id) through the library, get-or-create, so identity is stable. SourceRef::Local { path } is path-based, while canon-5cb2 wants content-addressed identity with the canon id embedded as a tag. Treat the Local variant as provisional so the wire format is not frozen around paths. Also: snapshots ship TrackRef.sources (local paths) to every client; decide whether remote clients should see bindings at all.

---
▸ 2026-09-23T21:09:15Z [Joel Webber]
Built: Source::track_meta replaced by describe(binding) -> SourceTrack (title, credited SourceArtists with service ids, SourceAlbum with id/cover art, disc/position, duration, ISRC); Tidal fills it from /v1/tracks (cover -> resources.tidal.com 640x640 URL). Store::ingest_track (atomic via savepoints): existing binding wins; else same ISRC = same recording, bound with provenance isrc; else create track + artists/album by their own bindings, place on album. Library::track_for(sources, binding) is the one door; canon-api play_track/enqueue use it; the client-minted `load` op is gone; EntityId::new documents that only the library mints. SourceRef::Local documented as provisional. Daemon opens <state_dir>/library.sqlite.

---
▸ 2026-09-23T21:09:15Z [Joel Webber]
Decision on the open question: snapshots keep carrying TrackRef.sources for now. Every client of this daemon is the user own (no auth, LAN only, and a client can already drive playback), and a UI wants to show where a track plays from. Revisit with auth or with a remote/shared deployment; recorded here rather than as a yak because nothing needs doing until then.

---
▸ 2026-09-23T21:09:15Z [Joel Webber]
On metal 2026-09-23: enqueue 55391792 55391792 55391790 -> queue ids 5c7ae60d.., 5c7ae60d.., 0ac14d2e.. (Money twice = one entity), meta from the library: Money / [Pink Floyd] / The Dark Side of the Moon / 380000 / artwork resources.tidal.com/images/05ccaf43/.../640x640.jpg; played locally (5, playing, Money 0->1393). Restarted daemon, enqueue 55391792 -> 5c7ae60d.. again (persisted). library.sqlite: 1 artist, 1 album (tracklist 1/4, 1/6), 2 tracks with ISRCs GBN9Y1100079/GBN9Y1100081, 4 direct bindings, user_version 1.

---
▸ 2026-09-23T21:09:16Z [Joel Webber]
verify: `cargo test -p canon-library && cargo test -p canon-api` -> PASS (exit 0)
