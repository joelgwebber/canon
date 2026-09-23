---
id: canon-1838
title: 'Engine sequence handoff: local gapless playback'
type: task
priority: 2
created: '2026-09-23T19:49:56Z'
updated: '2026-09-23T19:55:20Z'
parent: canon-77f8
labels:
- arch
- audio
- state
verify: cargo test -p canon-core --lib player
---

Step 2 of the canon-77f8 plan: continue into the next track on the same output instead of ending, starting with the local output (verifiable on the Mac speakers). Network flow mode builds on the same handoff and track-crossing event later.

DESIGN:
- The actor decides when to get the next track ready: on its position tick, while Playing on a frames-driven (local) output, within PRELOAD_LEAD of the end of a track with a known duration, and with a next entry. It emits Effect::Prepare { generation (current), next_generation, track }. Generations now come from one monotonic counter, so a prepared playback can never share a generation with a later start: a stale Described for it could otherwise be written into the wrong queue entry.
- The executor resolves the next track (meta: Described tagged next_generation, accepted for the prepared entry) and, if the same playback is still current and output is local, hands the input to the running engine: AudioPlayer::prepare_next(input, hint, to). The engine opens the decoder for it on a helper thread, so probing the next stream (a network fetch) never happens on the feed path, whose ring holds only 500ms.
- At EOF the engine takes the prepared decoder, if it is ready and has the same source rate and channels (the same device session and resampler), and keeps feeding. Otherwise it drains and ends as today (a format change is a break, per canon-77f8). It records the boundary in clock frames (start of track + frames fed). When the clock passes it, meaning the listener has crossed into the next track, the ENGINE rebases the clock (fetch_sub of the boundary, so no frames are lost to a race with the callback) and emits EngineEvent::Advanced { to }.
- The actor, on Advanced for its prepared playback: the current entry becomes the next one, generation = to, state stays Playing, and the queue revision moves. The controller forwarder re-tags later engine events with `to` and moves its latest generation along.
- Any start or halt (skip, seek, stop, sink change) clears the prepared playback. The engine is stopped, and its prepared input with it.
- Network outputs are unchanged here (no Prepare while a renderer drives position). Flow mode comes next.

EVIDENCE TO GET: on the local output, an entry change with no loading state and no device reopen between tracks, and position continuous across the boundary (N ends, N+1 from ~0). Then listen to a gapless album on the Mac.

---
▸ 2026-09-23T19:55:18Z [Joel Webber]
Done, as designed. Actor: maybe_prepare on the position tick (local output, Playing, within PRELOAD_LEAD=15s of a known end, next entry exists) emits Effect::Prepare. Generations come from one issued counter. Described is accepted for the prepared entry. EngineEvent::Advanced { to } makes the prepared entry current with no restart. start/halt clear the preparation. Engine: AudioPlayer::prepare_next opens the successor decoder on a helper thread. At EOF the feed loop joins it if it is the same source rate and channels (else a break: ended as before). The boundary is recorded in clock frames (fed), and cross_if_reached rebases the clock (FrameClock::rebase, a saturating fetch_update) and emits Advanced when the clock passes it; this is also flushed before Ended for a very short final track. Controller: executes Prepare (resolve meta + stream, hand it to the engine only if the same playback is still current and output is local), and the engine-event forwarder re-tags with `to` after Advanced and moves Inner.latest along.
Tests: the_next_entry_is_prepared_near_the_end_of_a_track, crossing_into_the_prepared_entry_moves_the_queue_on_without_a_restart, a_skip_abandons_the_prepared_entry.

On metal, local output (Mac speakers), enqueue 33348478 + 520285418 / seek 3:40 / sleep 22:
  15:53:23.946 (6 playing 0 Army of Me) 219660
  15:53:24.232 (7 playing 0 Army of Me)            <- the prepared entry described
  15:53:38.440 (7 playing 0) 234000 / 234000       <- the last snapshot of track 1, at its end
  15:53:38.684 (8 playing 1 Backstabber) 60        <- the next tick: track 2 at 60ms, no loading, no device reopen
  then 315, 559, 815 ... 2812 (continuous, real time)
Cast regression (no Prepare while a renderer drives): seek 3:44 -> (9 playing 0 ->233647) -> (10..13 loading -> playing 1 Backstabber 0->18027), no warnings.
Not yet verified: audible gaplessness on a gapless album. The snapshots show the join, but the ear is the real test (left to Joel).
Known limits: a device reopen mid-track loses the ring contents, so the join is reported up to ~0.5s late; the join check runs between decoded chunks (~100ms cadence).

---
▸ 2026-09-23T19:55:20Z [Joel Webber]
verify: `cargo test -p canon-core --lib player` -> PASS (exit 0)
