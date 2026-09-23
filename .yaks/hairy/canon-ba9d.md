---
id: canon-ba9d
title: 'Source seam that the controller actually uses: open-at-position, registry, resolver'
type: task
priority: 2
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T14:51:02Z'
parent: canon-4185
labels:
- arch
- library
---

From canon-ba30 (H). The controller holds Arc<TidalSession>, calls open_stream_at directly, picks tidal_id() off TrackRef, and fails any non-Tidal track. Source::resolve is a dead buffered fallback (fetch_all pulls the whole track into a Cursor) with no start position, which is why the controller could not use it. Change: Source::open(source, quality, start) -> (ResolvedStream, start_ms), a HashMap<Service, Arc<dyn Source>> registry in the daemon, and a resolver that walks TrackRef.sources by policy (local before streaming, per id.rs). Delete the buffered fallback. Also move codec_hint next to StreamInfo (duplicated 3x today). This is the seam canon-880e and canon-5cb2 plug into, and it is small now.
