---
id: canon-77f8
title: 'Design: gapless and crossfade on network outputs (device queue vs continuous stream)'
type: task
priority: 2
created: '2026-09-23T17:31:34Z'
updated: '2026-09-23T20:34:17Z'
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

---
▸ 2026-09-23T18:51:05Z [Joel Webber]
RESEARCH, 2026-09-23 (web; not yet verified on metal).
- A continuous stream is the established technique, not an exotic one. philippe44 LMS bridges (squeeze2upnp / squeeze2cast) have a "flow" mode that sends the whole playlist as one long stream "to enable true gapless and crossfade". The docs say that without flow mode gaps still happen "due to UPnP limitation", i.e. SetNextAVTransportURI does not reliably give true gapless across renderers. BubbleUPnP "Audio Cast" and Hi-Fi Cast do the same for Chromecast.
- Cast has no native gapless (Google issue tracker 36190694, "Add true gapless playback support to Chromecast Audio"). Gapless to Cast is achieved by the sender playing one continuous stream, so option A (device queue) is weak on Cast in practice.
- ICY metadata in practice: squeeze2upnp only offers it when re-encoding to MP3 (MP3/AAC), because players honour ICY for webradio codecs. Continuous FLAC therefore effectively means static metadata on DLNA displays, and lossy + ICY means titles on renderers that support ICY.
- No standard crossfade exists in UPnP AV or Cast. OpenHome (Linn and others) gives a renderer-owned playlist and gapless, not crossfade. Sonos crossfade is proprietary. Spotify/Tidal Connect crossfade is the service own receiver, not the protocol.
- Displays: the Cast Default Media Receiver shows the metadata sent with LOAD. In a continuous stream that stays static unless we ship a custom receiver app. The owner LS50s have no display, so metadata there is only for other control points.
IMPLICATION: B ("flow") is the well-trodden default for gapless + crossfade. A is the niche case (per-track metadata on display devices, at the cost of gaps and no crossfade). The choice looks like a per-output preference (flow on/off, plus codec for ICY), which would make it the first real setting for canon-f04a.

---
▸ 2026-09-23T19:06:38Z [Joel Webber]
DECISIONS (2026-09-23, with Joel):
1. Two per-output modes. FLOW (default): one continuous stream across tracks, giving gapless + crossfade on every renderer, with static device metadata (optional lossy+ICY titles later). STANDARD: one stream per track as today, with device metadata right for display devices, no gapless, no crossfade.
2. Device-mediated gapless (SetNextAVTransportURI, Cast queue) is dropped. canon-3aea slaughtered.
3. Queue edits inside the committed horizon, and sample-format changes between tracks, break the stream and restart at the audible position (the same mechanism as seek). A little jank at those edges is accepted.
4. A renderer reconnect mid-flow also breaks and restarts. We cannot control whether a renderer reconnects or what it reports afterwards. We serve the live edge on reconnect, so the audio it skipped would silently offset every boundary after it. We can observe the reconnect at our own stream server, and a fresh load makes the timeline correct by construction. Count only reconnects after the renderer has reported PLAYING for the load: the start-of-stream probe plus fetch (Rygel GET then GStreamer GET) is normal.

LAYERING: sinks and the stream server are unchanged by flow (one load per stream). The work is (a) an engine sequence handoff (continue into track N+1 on the same output when N is fully fed), which is also local gapless and the crossfade seam; and (b) a stream timeline in the player (track start offsets on the stream), mapping frame-clock or renderer position to the audible track, so a track change is a reality-driven event.
ORDER: canon-fdf3 (queue into the actor) -> engine sequence handoff (local gapless, verifiable on the Mac speakers) -> flow on network outputs with the per-output setting (the first real field for canon-f04a) -> canon-caae crossfade.
