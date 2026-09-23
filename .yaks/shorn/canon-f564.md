---
id: canon-f564
title: 'Flow mode: one continuous stream across tracks on network outputs'
type: task
priority: 2
created: '2026-09-23T20:10:21Z'
updated: '2026-09-23T20:23:59Z'
parent: canon-77f8
labels:
- arch
- sink
- audio
- state
verify: cargo test -p canon-core
---

Step 3 of canon-77f8. With an output in flow mode (the default, canon-f04a), consecutive tracks of one format join into the same stream, using the canon-1838 hand-off. The renderer never sees a track boundary, and we derive it from the renderer own reported position.

DESIGN:
- Generation = one playback RUN (a Start), not one track. A join inside a run does not change it, so renderer reports tagged with the load generation, including the stream final Ended, stay valid across joins. This simplifies canon-1838: the forwarder no longer re-tags and Inner.latest no longer moves on a join. The prepared entry keeps its own id (from the same counter) only to attribute its Described and its Advanced/Joined.
- Loaded gains `joins: bool` (the stream can take joins: local always, network iff flow). maybe_prepare keys on that instead of "no renderer".
- The network engine does the same EOF hand-off into the same FlacTap (same format required, else a break), and emits EngineEvent::Joined { to, at } with `at` = the stream time of the join (frames fed / rate). It cannot say when the join is HEARD; only the renderer knows.
- The actor turns `at` into a boundary on the current track timeline (via the RendererClock origin) and, on each report and tick, crosses when the renderer position reaches it: RendererClock::rebase(boundary), then the prepared entry becomes current. RendererClock origin becomes signed (after a join the stream began before this track did).
- Controller: the stream mode is read from settings at each network start, per output (the grouped output id, so one mode per speaker whatever the protocol). Prepare is honoured for a network output when its current stream joins. Renderer metadata stays the first track of the stream (accepted in canon-77f8).
- Not here: breaking on renderer reconnect (next yak).

EVIDENCE: on Tunes over Cast and DLNA, consecutive DSotM tracks in flow: one load, no loading between tracks, the entry changing near the boundary with the position continuing from ~0, the final Ended at queue end, and seek/skip/stop still correct. Standard mode unchanged.

---
▸ 2026-09-23T20:23:58Z [Joel Webber]
Done. A generation is now a RUN: joins do not change it, so the forwarder retag and Inner.latest moves from canon-1838 are gone. Loaded carries `joins` (local always; network iff flow), and maybe_prepare keys on it. The network engine joins a same-format successor into the same FlacTap and emits Joined { to, at } (the stream time of the join). The actor stores a pending join at clock.track_time(at) and crosses it on reports and ticks when the renderer position reaches it (RendererClock::rebase; the origin is signed). The controller reads the per-output mode from settings at each network start (keyed by the grouped output id) and honours Prepare on a joining stream.

Found on metal and fixed: RendererClock::rebase subtracted the boundary from the anchor, but the anchor is the position as of when the clock last started running (slews shift it without re-timing), so the next track appeared 23s in (Eclipse at 23094). It now re-anchors at now - boundary. Test crossing_on_a_running_clock_starts_the_next_track_near_zero (the earlier unit test used a clock that never ran). Also fixed: a new entry showed the previous track position while loading ("loading 127000/127000" for Eclipse); start() now resets position for a different entry. Test a_new_entry_reads_zero_while_it_loads.

On metal (Brain Damage 55391795 -> Eclipse 55391796, seek 3:30/3:35):
  CAST flow: (10 playing 0 Brain Damage ->230000) -> (11 playing 1 Eclipse) 92ms, then 104, 354, 604 ... Traces: joined at=22.814s boundary=230.493s; crossed at position 230.585s. No load at the join (loads: start + seek only).
  DLNA flow: (11 playing 1 Eclipse) 31ms, then 157, 407 ...; crossed at 230.524s. Stream connections: probe+fetch at start and at seek only.
  CAST flow transport: cross -> Eclipse 243ms; pause held (2279); play; prev -> new stream Brain Damage; next -> Eclipse; seek 1:58 -> (23 ended at 126997/127000). No warnings.
  CAST standard (mode Tunes standard): loads = 3 (start, seek, Eclipse), loading between tracks: unchanged behaviour.
  LOCAL regression: Eclipse at 57ms after the join; pause/play after the crossing work.

---
▸ 2026-09-23T20:23:59Z [Joel Webber]
verify: `cargo test -p canon-core` -> PASS (exit 0)
