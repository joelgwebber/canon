---
id: canon-f7da
title: Library is the only minter of EntityId; SourceRef::Local is provisional
type: task
priority: 2
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T14:51:16Z'
parent: canon-4185
depends_on:
- canon-78ea
labels:
- arch
- library
---

From canon-ba30 (I). canon-api play_track/enqueue build a TrackRef with EntityId::new() per call, so the same Tidal track enqueued twice is two entities. Once canon-78ea exists, the API must resolve (service, id) through the library, get-or-create, so identity is stable. SourceRef::Local { path } is path-based, while canon-5cb2 wants content-addressed identity with the canon id embedded as a tag. Treat the Local variant as provisional so the wire format is not frozen around paths. Also: snapshots ship TrackRef.sources (local paths) to every client; decide whether remote clients should see bindings at all.
