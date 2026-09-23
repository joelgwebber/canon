---
id: canon-685a
title: DLNA/UPnP control + GENA state feedback
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-23T16:56:00Z'
parent: canon-7718
depends_on:
- canon-bc84
- canon-583a
- canon-7c6f
labels:
- sink
- network
verify: cargo test -p canon-sink --lib
---

AVTransport SOAP (SetAVTransportURI/Play/Pause/Seek/GetTransportInfo). Unlike tideway, actually consume device state: GENA LastChange subscription (or a GetTransportInfo poll) fed back into the state machine, so device-side pause/skip is reflected. Prefer SetNextAVTransportURI for gapless where supported (tideway never used it). Version-agnostic service matching. Crate: rupnp.

---
▸ 2026-09-23T16:55:56Z [Joel Webber]
Done: DLNA plays end to end on Tunes (LS50 Wireless II, Rygel/GStreamer renderer, MediaRenderer:2 / AVTransport:2).
- Discovery: our own SSDP M-SEARCH (canon-sink::ssdp), bound to each usable interface with multicast egress pinned. rupnp/ssdp-client let the route table choose (tideway black hole), and rupnp eventing binds the first private IPv4. rupnp is used only for descriptions and SOAP, with default features off. Answers are believed for 3 missed rounds (93s) or the device max-age if shorter, and devices need an AVTransport, matched by type prefix (version-agnostic, and needed: it is :2 here).
- Sink (canon-sink::dlna): SetAVTransportURI with DIDL-Lite (audio/flac protocolInfo, which is in the LS50 GetProtocolInfo), then Play; Pause/Play/Stop; RenderingControl SetVolume (0-100) and SetMute. State is polled every 500ms: GetTransportInfo + GetPositionInfo, classified by a pure function. A foreign TrackURI means takeover, and STOPPED after PLAYING means Ended. RelTime is stream-relative whole seconds, read against the existing RendererClock origin: no second position model. Refused transport commands are logged, not fatal (UPnP 701 mid-transition). A failed load or 6 failed polls means Failed. A Stop is sent when the sink is dropped.
- Found on metal: flacenc STREAMINFO carried min frame size 0xFFFFFF (its no-frames tracker), and GStreamer flacparse waits for that many bytes, so it sat in TRANSITIONING forever. Fixed in the shared FLAC tap (min/max 0 = unknown). Test header_declares_frame_sizes_unknown. The first diagnostic was a python SSDP probe, which saw nothing at all: macOS Local Network privacy. Spike inside canon (noted in AGENTS.md).

On-metal evidence, sink dlna:uuid:c353baec-970d-4bf3-bb75-5c7c60bc2afe, enqueue 33348478 + 520285418 / sleep 15 / seek +30 / sleep 10 / pause / sleep 12 / play / sleep 6 / vol 3 / vol 4 / seek 3:44 / sleep 40 / stop:
  (4, playing, 0, Army of Me) pos 0->13459
  (7, playing, 0) pos 39938->48168          seek +30
  (8, paused, 0) pos 48192->48192           12s pause held
  (9..11, playing, 0) pos 48192->60151      resume, vol 3, vol 4 (device GetVolume afterwards: CurrentVolume 4)
  (14, playing, 0) pos 223654->233333       seek 3:44
  (15..17, playing, 1, Backstabber) 0->27847  auto-advanced on DLNA
  (18, idle)                                 stop
  RelTime polled: 0:00:00, 0:00:01, 0:00:02 ... (whole seconds, two polls each). No warnings.
Follow-ups: canon-3aea gapless (SetNextAVTransportURI), canon-2bb5 GENA eventing.

---
▸ 2026-09-23T16:56:00Z [Joel Webber]
verify: `cargo test -p canon-sink --lib` -> PASS (exit 0)
