---
id: canon-fa18
title: Show and log which binding the current track is streaming from
type: task
priority: 2
created: '2026-09-26T21:10:52Z'
updated: '2026-09-26T21:13:45Z'
parent: canon-c6ac
labels:
- state
- api
verify: cargo test -p canon-core the_snapshot_says && cargo test -p canon-core source
---

Asked 2026-09-26: with prefer spotify tidal, which service played? Nothing says. Sources::open records the binding it opened; the controller logs it (service, id, codec, rate, depth) and reports it to the actor, which carries it in the snapshot (playing_from), including across gapless/flow joins (the prepared entry's source applies when it is crossed into).

---
▸ 2026-09-26T21:13:44Z [Joel Webber]
Built: PlayingFrom {source, codec, sample_rate, bit_depth} (Display "tidal:36680688 (flac 16/44.1 kHz)"); ResolvedStream.from filled by Sources::open; EngineEvent::Streaming; the actor holds playing_from (cleared on start/halt, a prepared entry own applied on advance_into); PlayerSnapshot.playing_from; controller logs "playing <title> from <binding>" and "next up, <title>, from <binding>"; canon control status shows "· tidal flac 16/44.1"; Codec serializes snake_case.

---
▸ 2026-09-26T21:13:44Z [Joel Webber]
Live 2026-09-26 on :7399: prefer spotify tidal, queued imported Spotify playlist Prog at volume 0: "playing 0:04/4:34 The Lion's Roar — Cynic [1/14] · tidal flac 16/44.1", next -> "Kindly Bent to Free Us ... · tidal flac 16/44.1"; serve.log: playing "The Lion's Roar" from tidal:36680688 (flac 16/44.1 kHz). Confirms Joel's guess: with Spotify preferred but not streamable in the daemon, playback fell through to Tidal. Preference restored.

---
▸ 2026-09-26T21:13:45Z [Joel Webber]
verify: `cargo test -p canon-core the_snapshot_says && cargo test -p canon-core source` -> PASS (exit 0)
