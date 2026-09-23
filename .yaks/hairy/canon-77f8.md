---
id: canon-77f8
title: 'Design: gapless and crossfade on network outputs (device queue vs continuous stream)'
type: task
priority: 2
created: '2026-09-23T17:31:34Z'
updated: '2026-09-23T17:31:34Z'
labels:
- arch
- design
- sink
- audio
---

Design and research only. Decide how track boundaries work on network outputs, and capture the reasoning. No code until the decision is made.

TODAY: every boundary is controller-initiated. The track ends, then the controller loads the next, and the renderer starts it. Each load is its own stream whose body ends (canon-e920). So there is a gap on every output, local included: the engine drains the ring and reopens the device between tracks.

OPTION A: DEVICE QUEUE. DLNA SetNextAVTransportURI (supported by the LS50: its AVTransport:2 SCPD lists SetNextAVTransportURI / NextAVTransportURI / NextAVTransportURIMetaData). Cast receiver queue (QUEUE_LOAD / preload; rust_cast has load_with_queue, and the status already carries current_item_id / preloaded_item_id).
+ Per-track metadata on the device display is native and always right.
+ Per-track streams stay as they are, including mixed sample rates and bit-perfect hi-res per track.
- The renderer advances by itself. That needs a "renderer moved to load N+1" RendererEvent (today a new TrackURI reads as a takeover), which the controller follows rather than causes, and Sink::queue_next(url, meta) -> LoadId.
- It depends on per-device support and quality (the device zoo). Some renderers ignore SetNext, or gap anyway.
- Network crossfade is impossible: two separate URLs are played by the device, and neither protocol can overlap them. (A renderer with its own crossfade setting is outside our control.)

OPTION B: CONTINUOUS STREAM. Keep encoding track N+1 into the same stream, and record each boundary on the stream timeline. Renderer position is already stream-relative and read against an origin, so boundary crossings come from the renderer own reports: a small extension of RendererClock/player.
+ It behaves identically on every renderer, with no device feature needed.
+ Crossfade/DSP (canon-caae) happen before encoding, so they work on network outputs for free.
- One stream has one format. A mixed-rate queue must be resampled to a fixed stream format or break the stream at format changes, which costs bit-perfect hi-res across mixed queues.
- The device display shows one long item. Per-track metadata then needs a side channel: ICY metadata (StreamTitle) interleaved in the HTTP body, which GStreamer renderers request (the LS50 DLNA path sends icy-metadata: 1, unverified whether it displays it), or a custom Cast receiver app (a registered app id, which can take per-track metadata over a custom namespace). The Default Media Receiver probably ignores ICY (unverified).
- End-of-track detection moves back to our own boundary map, with the stream body ending only at the end of the queue or a format break.

SO METADATA VS CROSSFADE is exclusive only under A. Under B, crossfade is free and metadata becomes a per-protocol side channel of varying quality. HYBRID: B by default (break the stream at user skip/seek and format changes), and A as a per-sink mode when crossfade is off and device-native metadata matters. To be decided.

PREREQUISITES COMMON TO BOTH (worth doing regardless):
1. An engine next-track handoff: decode N+1 as soon as N is fully fed, into the same output. Needed for local gapless (drain+reopen today) and for crossfade.
2. The player represents a reality-driven track change as an EngineEvent, not a load command. It naturally lands with canon-fdf3 (queue in the actor), so do that first.
3. Pre-resolving the next queue entry ahead of time (overlaps canon-4400 cancellation semantics).

RESEARCH TO DO ON METAL: whether the LS50 DLNA path shows ICY StreamTitle; whether the Cast Default Media Receiver shows it for audio/flac; the actual SetNextAVTransportURI behaviour on the LS50 (true gapless or a gap); Cast QUEUE preload with Buffered FLAC; and what Kitchen/Basement (Google devices) do with each.
