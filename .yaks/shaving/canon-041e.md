---
id: canon-041e
title: Broken behavior streaming to Tunes@cast
type: bug
priority: 1
created: '2026-09-27T17:58:12Z'
updated: '2026-09-27T18:37:00Z'
labels:
- stream
- cast
---

When I attempt to stream to Tunes (over @cast, but haven't tested @dlna yet), it just starts jumping across tracks rapidly, playing nothing:

control:
```
loading  0:00/10:13  Their Waters Were Mingled Together — Austin Wintory  [22/480]
loading  0:00/10:13  Their Waters Were Mingled Together — Austin Wintory  [22/480]  · spotify vorbis 44.1
loading  0:14/10:13  Their Waters Were Mingled Together — Austin Wintory  [22/480]  · spotify vorbis 44.1
loading  0:00/1:08  Ori, Lost In the Storm (feat. Aeralie Brighton) — Gareth Coker  [23/480]
loading  0:00/1:08  Ori, Lost In the Storm (feat. Aeralie Brighton) — Gareth Coker  [23/480]  · spotify vorbis 44.1
loading  0:00/1:08  Ori, Lost In the Storm (feat. Aeralie Brighton) — Gareth Coker  [23/480]  · spotify vorbis 44.1
loading  0:00/1:24  Naru, Embracing the Light (feat. Rachel Mellis) — Gareth Coker  [24/480]
loading  0:00/1:24  Naru, Embracing the Light (feat. Rachel Mellis) — Gareth Coker  [24/480]  · spotify vorbis 44.1
loading  0:00/1:24  Naru, Embracing the Light (feat. Rachel Mellis) — Gareth Coker  [24/480]  · spotify vorbis 44.1
loading  0:00/3:28  The Blinded Forest — Gareth Coker  [25/480]
loading  0:00/3:28  The Blinded Forest — Gareth Coker  [25/480]  · spotify vorbis 44.1
loading  0:00/3:28  The Blinded Forest — Gareth Coker  [25/480]  · spotify vorbis 44.1
loading  0:00/1:22  Inspiriting — Gareth Coker  [26/480]
loading  0:00/1:22  Inspiriting — Gareth Coker  [26/480]  · spotify vorbis 44.1
loading  0:00/1:22  Inspiriting — Gareth Coker  [26/480]  · spotify vorbis 44.1
```

core:
```
2026-09-27T17:54:06.897630Z  INFO canon: player state seq=40 state=Loading position_ms=0
2026-09-27T17:54:06.930657Z  INFO canon: player state seq=41 state=Loading position_ms=0
2026-09-27T17:54:06.980906Z  INFO canon::controller: playing "To Know, Water" from tidal:63338696 (flac 16/44.1 kHz)
2026-09-27T17:54:06.981039Z  INFO canon: player state seq=42 state=Loading position_ms=0
2026-09-27T17:54:07.036581Z  INFO canon: player state seq=43 state=Loading position_ms=0
2026-09-27T17:54:07.602721Z  INFO canon_sink::cast: cast load accepted status=Status { request_id: 27, entries: [StatusEntry { media_session_id: 13, media: Some(Media { content_id: "http://192.168.0.67:63314/stream/13.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(13), media: Some(Media { content_id: "http://192.168.0.67:63314/stream/13.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }
2026-09-27T17:54:07.729377Z  INFO canon: player state seq=44 state=Loading position_ms=0
2026-09-27T17:54:07.903780Z  INFO canon::controller: playing "Seriola Lalandi" from tidal:63338698 (flac 16/44.1 kHz)
2026-09-27T17:54:07.903951Z  INFO canon: player state seq=45 state=Loading position_ms=0
2026-09-27T17:54:08.043057Z  INFO canon: player state seq=46 state=Loading position_ms=0
2026-09-27T17:54:08.341094Z  INFO canon_sink::cast: cast load accepted status=Status { request_id: 29, entries: [StatusEntry { media_session_id: 14, media: Some(Media { content_id: "http://192.168.0.67:63314/stream/14.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(14), media: Some(Media { content_id: "http://192.168.0.67:63314/stream/14.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }
2026-09-27T17:54:08.445652Z  INFO canon: player state seq=47 state=Loading position_ms=0
2026-09-27T17:54:08.535083Z  INFO canon::controller: playing "And the Earth Did Not yet Bear a Name" from tidal:63338699 (flac 16/44.1 kHz)
2026-09-27T17:54:08.535265Z  INFO canon: player state seq=48 state=Loading position_ms=0
2026-09-27T17:54:08.704697Z  INFO canon: player state seq=49 state=Loading position_ms=0
2026-09-27T17:54:09.120800Z  INFO canon_sink::cast: cast load accepted status=Status { request_id: 31, entries: [StatusEntry { media_session_id: 15, media: Some(Media { content_id: "http://192.168.0.67:63314/stream/15.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(15), media: Some(Media { content_id: "http://192.168.0.67:63314/stream/15.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }
2026-09-27T17:54:09.240825Z  INFO canon: player state seq=50 state=Loading position_ms=0
2026-09-27T17:54:09.453731Z  INFO canon::controller: playing "No Field Was Formed" from tidal:63338701 (flac 16/44.1 kHz)
2026-09-27T17:54:09.453927Z  INFO canon: player state seq=51 state=Loading position_ms=0
2026-09-27T17:54:09.599566Z  INFO canon: player state seq=52 state=Loading position_ms=0
2026-09-27T17:54:09.858002Z  INFO canon_sink::cast: cast load accepted status=Status { request_id: 33, entries: [StatusEntry { media_session_id: 16, media: Some(Media { content_id: "http://192.168.0.67:63314/stream/16.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(16), media: Some(Media { content_id: "http://192.168.0.67:63314/stream/16.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }
2026-09-27T17:54:09.898686Z  INFO canon: player state seq=53 state=Idle position_ms=0
2026-09-27T17:54:10.557283Z  WARN canon::controller: renderer session lost: sink: cast stop: an internal error occurred, Invalid request (INVALID_MEDIA_SESSION_ID). sink=SinkId("LS50-Wireless-II-d8bf502cfc52b2dc363a60e55e8cd967._googlecast._tcp.local.")
2026-09-27T17:54:10.558427Z  INFO canon: player state seq=54 state=Idle position_ms=0
...
```

---
▸ 2026-09-27T18:09:26Z [Joel Webber]
Diagnosed 2026-09-27. Root cause is the macOS Application Firewall, not canon's streaming: with canon_sink=trace the stream server logged no request at all, i.e. Tunes never reached http://192.168.0.67:<port>/stream/N.flac; the Cast accepted each LOAD, then its next status had no entries. socketfilterfw --listapps shows target/debug/canon 'Allow', but an ad-hoc signed binary's designated requirement is its cdhash (codesign -dr -: cdhash H"0a8b…"), which changes on every rebuild, so the rule no longer matches and inbound is dropped silently. Same cause hides DLNA: canon devices finds 4 Chromecasts (outbound + mDNS) and no DLNA endpoints (SSDP replies are inbound). Ruled out: IP/route (en0 192.168.0.67, Tunes pings), the 5305 seek change (seek is None at 0), the fa18 Streaming event (guarded, no prepares happened). Canon's own bug: cast::media_gone read 'media dropped' as the track finishing even when it never started, so each blocked load advanced the queue ~0.7s later, and autoplay kept refilling it. Fixed: a load dropped before the receiver ever buffered/played/paused is Failed(NEVER_PLAYED), which fails back to local with the reason. Live (blocked firewall, :7399, vol 0): one 'renderer session lost: the renderer dropped the stream without playing it…' then 'playing "Army of Me"' on local; no racing. Still owed: Joel re-allows the binary (sudo socketfilterfw), then an on-metal check that Tunes@cast plays and Tunes@dlna reappears; the durable fix is a stable signing identity (canon-f495 / canon-742b).

---
▸ 2026-09-27T18:37:00Z [Joel Webber]
Follow-ups built 2026-09-27 (Joel's answer to the close_notify drop and 'a more reliable error'): (1) StreamRoutes::ever_fetched; a lost session whose renderer never fetched a stream says so, naming the firewall; the consumer watchdog no longer calls that a takeover. (2) canon serve warns at startup when the firewall is on and the build is ad-hoc signed (codesign -dvv), and says how to grant a signed one. Live: signed build logged 'the macOS firewall is on; this canon is signed as "canon dev"…'. (3) RendererEvent::Disconnected: a dropped Cast control connection (status poll or heartbeat failing, e.g. the KEF's 'peer closed connection without sending TLS close_notify') reconnects to the same speaker once, restarting the track where it was, instead of falling back to local; a second drop within 30s, or a failed reconnect, falls back. Hypothesis, unproven: canon never answers the receiver's own PINGs (cast.rs step 3), which some receivers punish by closing the connection. Owed: on-metal on Tunes once Joel grants the signed build.
