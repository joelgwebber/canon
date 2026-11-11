---
id: canon-efa9
title: A renderer dropping a stream we are still feeding skips the rest of the track
status: shaving
type: bug
priority: 2
created: '2026-09-27T22:09:22Z'
updated: '2026-11-11T18:33:01Z'
tags:
- cast
verify: cargo test -p canon-daemon a_renderer_end && cargo test -p canon-core --lib a_failure_while_paused && cargo test --workspace
---

Seen during canon-9c73 (2026-09-27): Tunes@cast closed our HTTP stream mid-track (stream server: consumer gone ... ended_by_us=false) with our feed still running, then answered status polls with no entries. cast::media_gone reads 'media gone after it started' as RendererEvent::Ended, so the queue advanced ~2.5 minutes early. The stream had not been drained (StreamRoutes::is_drained false), so this was the renderer abandoning it, not the track ending. Probably: when media vanishes while our stream is not drained and the position is well short of the duration, restart the current track on the same renderer at the listener's position (like reconnect()), not Ended. canon-9c73 makes the drop much rarer (bigger renderer buffer) but doesn't handle it.

---
▸ 2026-11-11T18:28:51Z [delta-paused-drop]
kind: move
from: hairy
to: shaving
thread: ksQQi3CkeAFGRhaDgNRFBzZAF5LOAFfcwJIBxBCA0aLJyrhJPreUvVwLftHG
---

moved: hairy -> shaving

---
▸ 2026-11-11T18:33:01Z [delta-paused-drop]
kind: verify
result: pass
---

verify: `cargo test -p canon-daemon a_renderer_end && cargo test -p canon-core --lib a_failure_while_paused && cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-11-11T18:33:01Z [delta-paused-drop]
kind: checkpoint
commit: 208f97b
---

Built in `208f97b`; not sheared: the hardware check is still owed.

## The fix

- [`renderer_end`](code:crates/canon-daemon/src/controller.rs): a renderer's `Ended` becomes `EngineEvent::Ended` only if the session's newest stream was fed in full (`StreamRoutes::is_drained`). Otherwise it becomes `EngineEvent::Failed("the renderer dropped the stream before it was fed in full")`.
- The player's existing failure recovery does the rest: it restarts the track where the listener was, once per track (`recovered`), and skips it on a second failure. That is the `reconnect()`-like restart this yak asked for, with no new path.
- Flow mode: the tap lives across joins, so the stream counts as drained only when the whole run is fed. A mid-run drop restarts the current track.
- Paused: `EngineEvent::Failed` while `Paused` now holds the pause, like canon-c584's end, instead of restarting (which would play). This matters because a paused drop now arrives as a failure, not an end.
- `docs/device-quirks.md` updated for the KEF Cast entry.

## Evidence so far

- New tests: `a_renderer_end_is_the_track_ending_only_once_its_stream_was_fed`, `a_failure_while_paused_holds_the_pause_and_play_restarts_there`. canon-c584's tests still pass.
- Green bar: fmt, clippy (only the 2 dead-code warnings in `firewall.rs` already on `main`), `cargo test --workspace` and `cargo build --workspace` all pass.

## Still owed

On Tunes@cast, flow mode: wait for a c200 drop (2–14 min in) and confirm the log shows `restarting "…" where it was: the renderer dropped the stream…` with the same track continuing, not the next one. Also confirm that a normal end of a track still advances.
