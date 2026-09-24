---
id: canon-647a
title: 'Network loads lose their opening: chunks pushed before the renderer''s first GET are dropped'
type: bug
priority: 2
created: '2026-09-24T18:22:29Z'
updated: '2026-09-24T18:22:29Z'
labels:
- sink
- network
---

Found during canon-c200. The engine starts feeding the moment LOAD is sent, but the renderer's GET lands 20-800 ms later; StreamBroadcaster::push sends into a broadcast channel with no receivers, so those FLAC frames are discarded and every network load starts a fraction of a second into the track.

A first-consumer preroll (hold chunks until the first subscribe, replay after the header) was tried during canon-c200 (never verified by ear, so unproven even on Cast), and is wrong on DLNA: Rygel's probe GET (0 bytes, closes at once) is the first consumer and takes the preroll, so GStreamer's real fetch still starts at the live edge. Options: keep the preroll until a consumer has actually taken bytes past the header; or gate the feed (pace from the first real fetch) instead of buffering. Verify on metal on both protocols: first audible note of a track with an immediate attack.
