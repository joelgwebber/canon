---
id: canon-9c73
title: Tracks casting to Tunes@cast often stop streaming early; no sound until next track
type: bug
priority: 1
created: '2026-09-27T21:27:29Z'
updated: '2026-09-27T22:28:17Z'
labels:
- cast
verify: cargo test -p canon-core -p canon-audio -p canon-sink -p canon-daemon
---

I've noticed this quite a few times recently. No errors in the core logs, casting to Tunes (usually @cast). Everything acts like it's still playing the track -- again, no error logs; play head continues for the right amount of time; etc. When the next track starts, it continues like nothing was ever wrong.

I don't know whether I had the device set to standard or flow mode, so that might be worth checking (I honestly don't recall whether we ended up supporting flow mode on these sinks?).

---
▸ 2026-09-27T22:09:02Z [Joel Webber]
Repro (2026-09-27, Tunes@cast, flow mode, playlist #4 from #5): at 21:58:02 Tunes closed our HTTP stream itself (stream server: consumer gone ... ended_by_us=false) ~3:35 into Threshold (6:05), status went to entries: [] 0.4s later, media_gone read it as Ended and the queue advanced ~2.5 min early. ~25s before the drop the speaker's reported time ran at ~0.8x (lost 1.5s, 'renderer position snapped drift_ms=-1509'): starvation. Captured copy of the same stream (curl as a 2nd consumer) passes flac -t and ffmpeg decode, so the bytes were valid. Replaying Threshold: no drop at the same spot (not content).

---
▸ 2026-09-27T22:09:02Z [Joel Webber]
Cause: NETWORK_LEAD=2s means the renderer's buffer is only ~2-4s. Starvation episodes (reported time slipping 1.5-1.75s over ~10s) seen in every 2s-lead run: natural run end of track 1, Journey run pre-drop, Threshold replay at start. New 'network feed: stalled waiting on the source' warn (decode.next() > 250ms) never fired, so our feed was on time: delivery to the speaker hiccups and the thin buffer can't absorb it. 12s-lead run: speaker read 12s ahead happily (no ring lag), zero slips in 6 min. Flow-mode aggravator: the feed is paced from play_start and never re-anchors, so each slip is permanent extra lag.

---
▸ 2026-09-27T22:28:03Z [Joel Webber]
Fix: NETWORK_LEAD 2s -> 10s (the renderer's buffer against network hiccups), stream-server ring 64 -> 256 chunks (~24s, so a renderer that only reads at play rate never falls off it because we're ahead), PRELOAD_LEAD 15s -> 30s (the feed now runs ~10s ahead of the renderer position that preload is measured from; joins still prepared ~19s before the feed needs them). Kept the 'network feed: stalled waiting on the source' warn.

---
▸ 2026-09-27T22:28:03Z [Joel Webber]
Hardware evidence, fix build, Tunes@cast flow mode, playlist #4 from #5, 22:07-22:26: speaker lag behind wall clock per minute 0m:0.14 1m:0.10 2m:0.13 3m:0.14 4m:0.08 5m:0.06 6m:0.10 7m:0.07 8m:0.14 9m:0.10 10m:0.09 11m:0.09 12m:0.11 13m:0.10 14m:0.13 15m:0.08 16m:0.14 17m:0.11 18m:0.12. Zero 'snapped' slips, zero 'stalled', zero 'lagged', no renderer-initiated consumer drops; 8 flow joins all crossed within 0.2s of the boundary (e.g. 'listener crossed the join to=5 boundary=365.196s position=365.201s' = Threshold played in full). Baseline at 2s: 3 slips + 1 early drop in ~28 min. Transport on the fix build: seek +30, pause/play (held 0:04, resumed 0:04), next, seek 1:00 (landed 0:59), prev all correct.

---
▸ 2026-09-27T22:28:13Z [Joel Webber]
verify: `cargo test -p canon-core -p canon-audio -p canon-sink -p canon-daemon` -> PASS (exit 0)
