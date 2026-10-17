---
id: canon-5303
title: Quietly dropping cast connections without error or stopping
status: shaving
type: bug
priority: 2
created: '2026-10-04T15:27:26Z'
updated: '2026-10-12T14:44:02Z'
labels:
- network
---

Connection to Tunes@cast dropped for no obvious reason. Failed to reconnect, but playhead just kept chugging along. At the very least, a visible error would have been helpful.

```
2026-10-04T15:19:18.699533Z  INFO canon_sink::cast: cast load accepted status=Status { request_id: 5720, entries: [StatusEntry { media_session_id: 3, media: Some(Media { content_id: "http://192.168.0.27:7346/1/stream/3.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(3), media: Some(Media { content_id: "http://192.168.0.27:7346/1/stream/3.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }
2026-10-04T15:19:18.699686Z  INFO canon_sink::stream_server: stream server: consumer connected, replaying header + live edge
2026-10-04T15:19:29.415183Z  WARN canon::controller: renderer connection dropped (cast status poll: failed to fill whole buffer); reconnecting sink=SinkId("LS50-Wireless-II-d8bf502cfc52b2dc363a60e55e8cd967._googlecast._tcp.local.")
2026-10-04T15:19:29.415497Z  INFO canon_sink::stream_server: stream server: consumer gone served=625424 secs=10.715779214 ended_by_us=true
2026-10-04T15:19:31.694610Z  WARN canon::controller: reconnecting failed (sink: cast connect 192.168.0.205:8009: Connection refused (os error 111)); failing back to local sink=SinkId("LS50-Wireless-II-d8bf502cfc52b2dc363a60e55e8cd967._googlecast._tcp.local.")
```

---
▸ 2026-10-04T16:23:57Z [Joel Webber]

Diagnosis 2026-10-04. The drop is the speaker's doing: 'failed to fill whole buffer' is read_exact hitting a clean EOF, which rustls only gives after the peer's TLS close_notify (a bare TCP close reads 'peer closed connection without sending TLS close_notify', as canon-041e saw). So Tunes closed the session deliberately, and 2.3s later nothing was listening on :8009 at all (Connection refused): its Cast service was down or restarting. Next time it happens, time the outage from this machine: while true; do printf '%s ' "$(date +%T)"; nc -z -w1 192.168.0.205 8009 && echo up || echo down; sleep 1; done -- that says how long reconnect has to keep trying. Also note whether Tunes went to standby or changed source. The silence is canon's: see the children of this yak.
---
▸ 2026-10-12T14:44:02Z [Joel Webber]

New examples from the logs:

Repeatedly stopped playing on cast, dropping to local:
```
2026-10-12T14:39:59.786592Z  INFO canon: player state seq=474 state=Playing position_ms=41714
2026-10-12T14:39:59.808591Z  INFO canon: player state seq=475 state=Loading position_ms=0
2026-10-12T14:39:59.867683Z  INFO canon::controller: playing "Await Rescue" from tidal:8829345 (aac 0 kHz)
2026-10-12T14:39:59.867886Z  INFO canon: player state seq=476 state=Loading position_ms=0
2026-10-12T14:39:59.903411Z  INFO canon: player state seq=476 state=Loading position_ms=0
2026-10-12T14:39:59.918645Z  INFO canon: player state seq=477 state=Loading position_ms=39938
2026-10-12T14:40:00.042854Z  INFO canon_sink::cast: cast load accepted status=Status { request_id: 3, entries: [StatusEntry { media_session_id: 1, media: Some(Media { content_id: "http://192.168.0.67:7346/10/stream/1.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(1), media: Some(Media { content_id: "http://192.168.0.67:7346/10/stream/1.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }
2026-10-12T14:40:00.061318Z  INFO canon_sink::stream_server: stream server: consumer connected, replaying header + live edge
2026-10-12T14:40:10.830206Z  WARN canon::controller: renderer connection dropped (cast status poll: failed to fill whole buffer); reconnecting sink=SinkId("LS50-Wireless-II-d8bf502cfc52b2dc363a60e55e8cd967._googlecast._tcp.local.")
2026-10-12T14:40:10.830745Z  INFO canon_sink::stream_server: stream server: consumer gone served=1446698 secs=10.769390584 ended_by_us=true
2026-10-12T14:40:13.341308Z  INFO canon: player state seq=478 state=Loading position_ms=39938
2026-10-12T14:40:13.341315Z  WARN canon::controller: reconnecting failed (sink: cast connect 192.168.0.205:8009: Connection refused (os error 61)); failing back to local sink=SinkId("LS50-Wireless-II-d8bf502cfc52b2dc363a60e55e8cd967._googlecast._tcp.local.")
2026-10-12T14:40:13.398167Z  INFO canon::controller: playing "Await Rescue" from tidal:8829345 (aac 0 kHz)
2026-10-12T14:40:13.398254Z  INFO canon: player state seq=479 state=Loading position_ms=39938
```

Then when I reconnected over DLNA and got it to start again by advancing tracks:
```
2026-10-12T14:40:40.669661Z  INFO canon: player state seq=481 state=Loading position_ms=0
2026-10-12T14:40:40.726905Z  INFO canon::controller: playing "Await Rescue" from tidal:8829345 (aac 0 kHz)
2026-10-12T14:40:40.727205Z  INFO canon: player state seq=482 state=Loading position_ms=0
2026-10-12T14:40:40.744012Z  INFO canon: player state seq=482 state=Loading position_ms=0
2026-10-12T14:40:40.752477Z  INFO canon: player state seq=483 state=Loading position_ms=59907
2026-10-12T14:40:40.889597Z  INFO canon_sink::cast: cast load accepted status=Status { request_id: 3, entries: [StatusEntry { media_session_id: 1, media: Some(Media { content_id: "http://192.168.0.67:7346/12/stream/1.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(1), media: Some(Media { content_id: "http://192.168.0.67:7346/12/stream/1.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }
2026-10-12T14:40:40.901194Z  INFO canon_sink::stream_server: stream server: consumer connected, replaying header + live edge
2026-10-12T14:40:42.094099Z  INFO canon: player state seq=484 state=Loading position_ms=0
2026-10-12T14:40:42.114777Z  INFO canon_sink::stream_server: stream server: consumer gone served=761584 secs=1.213566541 ended_by_us=true
2026-10-12T14:40:42.154186Z  INFO canon::controller: playing "Verses from Our Captain" from tidal:27136873 (flac 16/44.1 kHz)
2026-10-12T14:40:42.154291Z  INFO canon: player state seq=485 state=Loading position_ms=0
2026-10-12T14:40:42.407437Z  INFO canon: player state seq=486 state=Loading position_ms=0
2026-10-12T14:40:42.453323Z  WARN canon::controller: renderer connection dropped (cast status poll: peer closed connection without sending TLS close_notify: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof); reconnecting sink=SinkId("LS50-Wireless-II-d8bf502cfc52b2dc363a60e55e8cd967._googlecast._tcp.local.")
2026-10-12T14:40:44.789289Z  INFO canon: player state seq=487 state=Loading position_ms=0
2026-10-12T14:40:44.789295Z  WARN canon::controller: reconnecting failed (sink: cast connect channel: Connection reset by peer (os error 54)); failing back to local sink=SinkId("LS50-Wireless-II-d8bf502cfc52b2dc363a60e55e8cd967._googlecast._tcp.local.")
2026-10-12T14:40:44.838037Z  INFO canon::controller: playing "Verses from Our Captain" from tidal:27136873 (flac 16/44.1 kHz)
2026-10-12T14:40:44.838094Z  INFO canon: player state seq=488 state=Loading position_ms=0
2026-10-12T14:40:44.917320Z  INFO canon: player state seq=489 state=Playing position_ms=0
2026-10-12T14:40:45.036625Z  INFO canon: player state seq=489 state=Playing position_ms=104
2026-10-12T14:40:45.287465Z  INFO canon: player state seq=489 state=Playing position_ms=359
2026-10-12T14:40:45.535852Z  INFO canon: player state seq=489 state=Playing position_ms=603
2026-10-12T14:40:45.787137Z  INFO canon: player state seq=489 state=Playing position_ms=859
2026-10-12T14:40:46.037673Z  INFO canon: player state seq=489 state=Playing position_ms=1102
2026-10-12T14:40:46.286769Z  INFO canon: player state seq=489 state=Playing position_ms=1358
2026-10-12T14:40:46.537212Z  INFO canon: player state seq=489 state=Playing position_ms=1602
2026-10-12T14:40:46.785976Z  INFO canon: player state seq=489 state=Playing position_ms=1857
2026-10-12T14:40:47.041023Z  INFO canon: player state seq=489 state=Playing position_ms=2113
2026-10-12T14:40:47.287211Z  INFO canon: player state seq=489 state=Playing position_ms=2356
2026-10-12T14:40:47.537165Z  INFO canon: player state seq=489 state=Playing position_ms=2612
2026-10-12T14:40:47.786790Z  INFO canon: player state seq=489 state=Playing position_ms=2856
2026-10-12T14:40:48.037747Z  INFO canon: player state seq=489 state=Playing position_ms=3111
2026-10-12T14:40:48.173111Z  INFO canon: player state seq=490 state=Loading position_ms=0
2026-10-12T14:40:48.228149Z  INFO canon::controller: playing "Verses from Our Captain" from tidal:27136873 (flac 16/44.1 kHz)
2026-10-12T14:40:48.228309Z  INFO canon: player state seq=491 state=Loading position_ms=0
2026-10-12T14:40:48.257327Z  INFO canon: player state seq=491 state=Loading position_ms=0
2026-10-12T14:40:48.262629Z  INFO canon: player state seq=492 state=Loading position_ms=0
2026-10-12T14:40:48.300246Z  INFO canon_sink::stream_server: stream server: consumer connected, replaying header + live edge
2026-10-12T14:40:48.300271Z  INFO canon_sink::stream_server: stream server: consumer gone served=0 secs=1.275e-5 ended_by_us=false
2026-10-12T14:40:48.445837Z  INFO canon_sink::stream_server: stream server: consumer connected, replaying header + live edge
2026-10-12T14:40:48.766264Z  INFO canon: player state seq=492 state=Loading position_ms=0
2026-10-12T14:40:49.268681Z  INFO canon: player state seq=493 state=Playing position_ms=0
2026-10-12T14:40:49.286840Z  INFO canon: player state seq=493 state=Playing position_ms=18
2026-10-12T14:40:49.535863Z  INFO canon: player state seq=493 state=Playing position_ms=267
```
