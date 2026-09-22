---
id: canon-4c55
title: 'Stream resolution: MPD → segment list + manifest cache'
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T02:01:38Z'
parent: canon-94cc
labels:
- tidal
---

playbackinfo returns a base64 DASH/MPD manifest. Do our OWN MPD → segment-URL extraction (init at index 0, media 1..N fragmented-MP4); do NOT feed Tidal's MPD to a generic DASH demuxer (libav rejects it across tiers). Reject is_encrypted manifests. Cache resolved manifests by (track_id, quality) with a ~180s TTL sized to the signed-URL lifetime. Crates: quick-xml or dash-mpd.
