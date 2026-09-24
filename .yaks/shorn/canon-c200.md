---
id: canon-c200
title: Losing network sink stream partway through some tracks
type: bug
priority: 1
created: '2026-09-24T15:40:56Z'
updated: '2026-09-24T18:48:47Z'
verify: 'grep -q "Cast: drops our live stream" docs/device-quirks.md && cargo test -p canon-sink --quiet'
---

I've seen this a few times while streaming a playlist to a network device (Tunes: chromecast). The playlist works normally across several tracks, then eventually stops playing; neither the `serve` nor `control` logs show anything amiss when this happens. Then when the _next_ track picks up, it starts playing properly again, but also shows these two INFO logs (about the time the track changes and picks up again):

```
INFO canon_sink::cast: cast load accepted status=Status { request_id: 832, entries: [StatusEntry { media_session_id: 2, media: Some(Media { content_id: "http://192.168.0.67:60706/stream/2.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }), playback_rate: 1.0, player_state: Idle, current_item_id: Some(1), loading_item_id: None, preloaded_item_id: None, idle_reason: None, extended_status: Some(ExtendedStatus { player_state: Loading, media_session_id: Some(2), media: Some(Media { content_id: "http://192.168.0.67:60706/stream/2.flac", stream_type: Buffered, content_type: "audio/flac", metadata: None, duration: None }) }), current_time: Some(0.0), supported_media_commands: 274447 }] }

INFO canon_sink::stream_server: stream server: consumer connected, replaying header + live edge
```

These logs don't look problematic in themselves, but I only see them on a track change after this gap. The gap seems to be of arbitrary length: when it stops playing, it never picks up again until a track change.

---
▸ 2026-09-24T16:42:22Z [Joel Webber]
Repro'd 4x on Tunes (Cast, flow mode) with debug build, ~2-14 min in, any track (Army of Me @112s, Black Steel twice, Sour Times @173s). Signature: renderer keeps reporting PLAYING but current_time advances ~0.8x for ~20-25s, then the renderer itself closes our HTTP connection (new 'consumer gone ... ended_by_us=false' log), status goes empty, we read that as Ended and LOAD the next track (stream/N+1 = the user's two INFO lines). Ruled out: ring lag (new WARN never fired, backlog=0), feed behind realtime (never), Tidal segment fetch (Black Steel fetches all 86 segs in 2-4s x3), FLAC corruption (curl tap of same stream: 414s, flac -t / ffmpeg clean). Hypothesis under test: Wi-Fi throughput dip to the KEF + only ~2s NETWORK_LEAD cushion -> renderer underruns and gives up. Sampling netstat Send-Q + ping to 192.168.0.205.

---
▸ 2026-09-24T18:02:04Z [Joel Webber]
Runs 5-8 (2026-09-24): Send-Q to KEF ~0 and ping 4-43ms through a crawl -> not Wi-Fi. 30s NETWORK_LEAD + first-consumer preroll (run 6): no crawl, but renderer still quit abruptly 3x in 40min with ~30s buffered -> not buffer depth; consistent with the KEF's Cast player stalling its reader at T and erroring ~25-30s later (2s lead = silence+crawl the user hears). Receiver app/session unchanged across a drop (run 7). Spotify desktop held a Cast conn to Tunes; quitting it didn't stop drops. DLNA to Tunes, same stream + code: ~25 min clean incl. 4 flow joins. User: Tidal app / Tideway casting to KEF never showed this. Next: E1 Cast with 5s polls (CANON_CAST_POLL_MS), E2 Library display (Google) over Cast, E3 finite Content-Length FLAC via temp 'canon cast-file'. Side finding: pushes before the first GET are dropped (start of every network load lost); preroll fix conflicts with Rygel's probe connection -> needs its own yak.

---
▸ 2026-09-24T18:29:39Z [Joel Webber]
Moved to branch c200-kef-cast (worktree ../canon-c200) so main is free for library work. E1 (Tunes Cast, 5s polls): dropped at ~5 min -> poll rate ruled out. E2 (Library display, Cast, flow): ~26 min clean, 6 joins, until the user paused it. E3 (Tunes Cast, finite 55-min FLAC w/ Content-Length + ranges via temp cast-file harness): the KEF reads in ~9.4 MB range bursts, hanging up and returning with Range every ~80-100 s; clean past 21 min so far.

---
▸ 2026-09-24T18:48:26Z [Joel Webber]
RESOLUTION: diagnosed, documented, not fixed in code (user's call: not worth forcing unless simple). Cause: the KEF LS50 Wireless II's Cast player ('Cast Lite') reads a finite resource in ~9.4 MB range bursts, hanging up and resuming with Range; our live chunked stream (no Content-Length, no Accept-Ranges) can't be resumed that way, and 2-14 min in playback stalls and the player abandons it, which we read as Ended -> skip. Evidence: E3 finite FLAC with Content-Length + ranges to Tunes over Cast: 2393 s (40 min), 28 range GETs, no Ended/Failed event; live stream to Tunes over Cast never passed ~806 s across 9 drops in runs 1-7 + E1. Controls: DLNA to Tunes 1422 s / 5 joins clean; Nest Hub (Library display) over Cast 1729 s / 6 joins clean. Shipped: docs/device-quirks.md (seeded with every device lesson found in code comments + yaks); WARN on ring lag (was silent); 'consumer gone ... ended_by_us' info line (the only trace this drop leaves); WARN on Tidal segment fetch failure / reader error. Follow-ups: canon-07b6 (structured quirks: warn / prefer DLNA for Tunes), canon-647a (lost opening of every network load). Green bar: fmt ok, clippy 0 warnings, 191 tests passed / 0 failed, build ok, doctor --strict clean. Experiment scaffolding (cast-file harness, poll override, receiver-status log, 30 s lead, preroll) saved outside the repo in canon/target/c200-experiments.patch and target/c200-castfile.rs.

---
▸ 2026-09-24T18:48:36Z [Joel Webber]
verify: `grep -q "Cast: drops our live stream" docs/device-quirks.md && cargo test -p canon-sink --quiet` -> PASS (exit 0)

---
▸ 2026-09-24T18:48:47Z [Joel Webber]
Correction to the RESOLUTION note: 10 live-stream drops, not 9 (runs 1, 2, 4, 7 one each; run 5 two; run 6 three; E1 one).
