---
id: canon-bc84
title: Accurate cast position from MEDIA_STATUS (reconcile frames-fed vs reported)
type: task
priority: 2
created: '2026-09-23T00:24:18Z'
updated: '2026-09-23T04:09:03Z'
parent: canon-7718
labels:
- sink
verify: cargo test -p canon-core --lib && cargo test -p canon-sink --lib cast
---

v1 drives position from frames fed to the encoder, which leads actual Cast playback by the Cast buffer (~1-3s). Reconcile the FrameClock to the Cast MEDIA_STATUS reported position (seek clock on each status, free-run by wall-time between). Needs a wall-time clock advance on the network path.

---
▸ 2026-09-23T03:15:40Z [Joel Webber]
SEQUENCING DECISION (user instinct, endorsed): do this BEFORE canon-685a (DLNA). It is not a Cast detail — it is 'where does playback position come from when a NETWORK renderer is playing', which is shared infrastructure every network sink needs. Settle the authority once and DLNA inherits it; do DLNA first and we end up reconciling two half-baked position models. (User: pulling stream position from the wrong place was a source of unending pain in tideway — same class of bug.)
CURRENT STATE: on the network path the engine calls clock.advance(frames_fed), so position tracks what we have ENCODED AND SENT, which leads the audio the renderer is actually playing by its buffer depth (~1-3s). Worse, it is open-loop: nothing ever corrects drift.
WHAT WE ALREADY HAVE: the Cast status poll returns StatusEntry.current_time (observed live, e.g. current_time: Some(76.34827)) every ~500ms. DLNA's equivalent is AVTransport GetPositionInfo/RelTime (plus GENA events). So both protocols can report authoritative position — the seam should take 'renderer-reported position' generically, not Cast-specific.
DESIGN SHAPE TO SETTLE:
- FrameClock is frames+epoch+sample_rate and is designed to be advanced by the RT callback. There is no RT callback on the network path, so it needs a wall-time advance mode (free-run between reports) plus correction from the renderer's reported position.
- Do NOT hard-seek on every report: at 2/sec that would visibly jitter the position. Prefer a drift threshold (only correct when |reported - derived| exceeds e.g. 500ms) or slew, so normal playback is smooth and only real divergence snaps.
- Watch the epoch contract: seek() bumps epoch (clients treat it as a discontinuity). A routine position correction is NOT a user-visible discontinuity, so it likely needs a distinct 'reconcile' path that does not bump epoch, or clients will see constant seeks.
- Carry it as a new generic event (e.g. CastEvent::Position(ms) -> a controller-level 'renderer position' input) so canon-685a plugs into the same path.

---
▸ 2026-09-23T03:48:26Z [Joel Webber]
DONE. Position on a network renderer is now the renderer's own report, extrapolated by wall time between reports and reconciled continuously. Shape: PositionDrive{Frames,Renderer} rides on EngineEvent::Loaded (settled per stream open, which is also per output, since switching outputs restarts the stream); RendererClock (canon-core/src/position.rs) is an anchor+stopwatch that only runs while Playing; EngineEvent::RendererPosition folds reports in. Deliberately generic, NOT Cast-shaped: canon-685a feeds AVTransport GetPositionInfo/RelTime into the same event.

All three design traps addressed. (1) Wall-time free-run: RendererClock anchor + Instant, no RT callback needed. (2) No hard-seek per report: corrections slew (drift/4 when the renderer is ahead, drift/8 when behind -- gentler backward, because 'behind' is what a coarse-reporting renderer looks like; DLNA RelTime's whole-second truncation reads as a permanent half-second lag). Only a divergence over RendererClock::SNAP (1.5s) snaps. (3) Epoch/seq contract: a reconcile is handled BEFORE the generic input path, so it never bumps seq and never touches FrameClock::seek -- clients cannot read routine reconciliation as the user seeking twice a second.

Also fixed en route, all fallout from the same root cause: (a) a renderer stream now reports Loading until the device says it is playing, instead of claiming Playing the moment we hand it a URL and running position ahead through the buffering; (b) device reports enter as EngineEvent::RendererState, not Command::Play/Pause -- a command is user intent, and laundering device status through it left the state machine unable to tell them apart; (c) mid-track rebuffering freezes position instead of free-running; (d) select_sink / fail_back_to_local resume at the position actually heard, so leaving a cast no longer jumps ~4s forward.

---
▸ 2026-09-23T03:48:35Z [Joel Webber]
ON-METAL EVIDENCE (KEF 'Tunes' 192.168.0.205, Tidal 520285418).

BEFORE, measured with 'canon cast' (which now prints both numbers side by side):
  [device] at  0.0s (we have fed  3.6s - +3.6s ahead)
  [device] at 44.2s (we have fed 48.1s - +3.9s ahead)
  [device] at 45.9s (we have fed 49.9s - +4.0s ahead)
A steady ~3.8s lead (2s pacing lead + ~1.8s receiver buffer) that never self-corrects -- exactly the open-loop error this yak was filed for, now quantified.

AFTER, 'canon serve' + a control-plane client, RUST_LOG=canon_core::player=trace:
  renderer position reconciled derived_ms=1199  reported_ms=964   by_ms=-29
  renderer position reconciled derived_ms=39149 reported_ms=38965 by_ms=-23
  renderer position reconciled derived_ms=42067 reported_ms=41876 by_ms=-26
72 consecutive reports over ~42s, ZERO snaps: derived position tracks the device within ~200ms and stays there. The residual ~200ms is about the report's own sampling+transit latency, so true error is smaller still; compensating for it would need a round-trip estimate and is not worth it yet.

Published snapshots held seq=4 for the whole of playback across all 72 corrections, and state went idle -> loading -> playing (device-driven). The seq contract survives continuous reconciliation, which was trap (3).

---
▸ 2026-09-23T03:48:38Z [Joel Webber]
verify: `cargo test -p canon-core --lib && cargo test -p canon-sink --lib cast` -> PASS (exit 0)

---
▸ 2026-09-23T04:03:42Z [Joel Webber]
REGROWN: the claim 'accurate cast position' does not survive a seek. Found within minutes of having canon-444b's scriptable client, by piping 'sink Tunes / enqueue 33348478 / sleep 25 / seek +30 / sleep 8 / pause / play' at the daemon. Two distinct defects, both invisible to steady-state playback:

(1) RENDERER REPORTS ARE STREAM-RELATIVE, NOT SOURCE-TIMELINE. Seeking restarts the stream at the seek point (open_stream_at is segment-granular and returns start_ms), and a Cast receiver reports current_time relative to the media it was handed -- so after seeking to 0:47 the device correctly reported 0:00, 0:04, 0:06 and reconcile dragged our position down with it. Observed: 'playing 0:21' -> seek -> 'loading 0:47' -> 'loading 0:04'. RendererClock must know the stream's origin on the source timeline and add it to every report. This generalises: DLNA RelTime is likewise relative to the current track URI, so the origin belongs in the clock, not in Cast-specific glue.

(2) THE CAST STATE DEDUP CACHE OUTLIVES THE SESSION IT DESCRIBES. report() suppresses a state equal to the last one it forwarded, but after a re-LOAD the player is back to Loading while the cast thread still believes it has already reported Playing -- so the device's Playing is swallowed and the player stays wedged in Loading forever (seen above: it never left 'loading', and pause/play could not rescue it). Pre-existing, but harmless until bc84 made a renderer stream start in Loading rather than claiming Playing outright. The same hole hides a failed pause: if the device ignores a pause we send, the player believes Paused and no contradicting Playing is ever forwarded.

Fix direction: level-triggered device states (Playing/Paused/Buffering) must always flow, and the PLAYER decides what counts as a transition -- that is where the knowledge of its own state lives. Only edge-triggered events (Ended/Superseded/Failed) need dedup, since those drive queue auto-advance and must fire once.

---
▸ 2026-09-23T04:08:56Z [Joel Webber]
FIXED, both defects, re-verified on the KEF with the same pipeline that found them.

(1) RendererClock now carries the stream's origin on the source timeline and reads every report against it, so 'four seconds into the stream you were given' resolves to 0:51 of a track when that stream began at 0:47. seek() moves the origin with the position, since seeking a renderer means handing it a fresh stream starting there -- leave the origin behind and the next report reads as a jump to the top. Generic, not Cast-specific: DLNA RelTime is relative to the current track URI in exactly the same way.

(2) Level vs edge in the cast report path. Playing/Paused/Buffering describe a condition and now always flow; only Ended/Superseded/Failed dedup, because those drive one-shot actions (auto-advance, fail-back). The player decides what counts as a transition -- EngineEvent::RendererState bumps seq only when the state actually differs -- which is the correct home for that judgement, since the player is the only thing that knows its own state. handle() now returns Transition so that decision is explicit per input rather than 'everything is a transition'.

AFTER (same command that produced the failure):
  playing  0:18 -> seek +30 -> loading 0:18 -> loading 0:47 -> playing 0:47 -> 0:48, 0:50, 0:51 ...
  pause -> paused 0:58 -> play -> playing 0:58 -> 1:00, 1:01 ...
Position lands on the seek target and keeps counting, the state leaves Loading on the device's say-so, and pause freezes rather than drifting.

Worth remembering: steady-state playback looked perfect for 42s and hid both of these. It took a transport action (seek) against real hardware to expose them, which is the argument for canon-444b existing at all.

---
▸ 2026-09-23T04:09:03Z [Joel Webber]
verify: `cargo test -p canon-core --lib && cargo test -p canon-sink --lib cast` -> PASS (exit 0)
