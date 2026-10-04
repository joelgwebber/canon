---
id: canon-5303
title: Quietly dropping cast connections without error or stopping
type: bug
priority: 2
created: '2026-10-04T15:27:26Z'
updated: '2026-10-04T15:27:26Z'
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
