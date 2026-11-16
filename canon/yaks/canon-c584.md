---
id: canon-c584
title: A renderer dropping a paused stream skips the track and plays the next
status: shaving
type: bug
priority: 1
created: '2026-11-11T18:16:03Z'
updated: '2026-11-11T18:22:39Z'
tags:
- cast
- sink
verify: cargo test -p canon-core --lib an_end_ && cargo test --workspace
---

Seen 2026-11-11 on Tunes@cast, flow mode, driven by `canon player` on the Mac against Joel's daemon on j15r.

## What happens

1. The listener pauses. The engine's feed pauses with it, so the KEF stops getting bytes.
2. About 40 s later the KEF closes our HTTP stream itself (`stream server: consumer gone … ended_by_us=false`). A 17 s pause survived; every pause of 40 s or more did not.
3. The Cast status comes back empty, and `cast::media_gone` reads that as `RendererEvent::Ended`.
4. `EngineEvent::Ended` is honoured while `Paused` (`in_play()` includes it), so the queue advances, and `Effect::Start` plays the next track. The listener paused; canon played.

Pausing the new track restarts the countdown, so it cycled through five tracks, one every ~43 s (12:34–12:37 local).

## The rule

A paused track can't end: nothing is playing. An `Ended` while paused means the output lost the stream. The player holds the pause and its place, and a later play restarts the track there. That holds for any output, so it belongs in the player.

The executor's consumer watchdog also has to leave a paused renderer alone. Otherwise a dropped paused stream reads, 10 s later, as a takeover and fails back to local, which plays too.

## Related

- canon-efa9: the same misreading mid-play (a drop with our feed still running). Not covered here.
- canon-c200: the KEF's Cast drops of our live stream generally.

---
▸ 2026-11-11T18:16:24Z [delta-paused-drop]
kind: move
from: hairy
to: shaving
thread: ksQQi3CkeAFGRhaDgNRFBzZAF5LOAFfcwJIBxBCA0aLJyrhJPreUvVwLftHG
---

moved: hairy -> shaving

---
▸ 2026-11-11T18:22:39Z [delta-paused-drop]
kind: verify
result: pass
---

verify: `cargo test -p canon-core --lib an_end_ && cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-11-11T18:22:39Z [delta-paused-drop]
kind: checkpoint
commit: b458af8
---

Built in `b458af8`; not sheared: the hardware check is still owed.

## The fix

- [`EngineEvent::Ended`](code:crates/canon-core/src/player.rs) while `Paused` no longer advances. It sets `stream_lost` and logs `the output dropped paused "…"; play restarts it where it was`.
- `Command::Play` with `stream_lost` restarts the same entry at the held position (`restart_here`) rather than sending `Resume` to a stream that no longer exists. Any start or halt clears the flag.
- The consumer watchdog's `loaded()` is false while paused, so a dropped paused stream isn't read as a takeover 10 s later (which would fail back to local and play there).

## Evidence so far

- New tests: `an_end_while_paused_holds_the_pause_and_play_restarts_there`, `an_end_after_resuming_advances_as_usual`. Both pass.
- Green bar: fmt, clippy (no new warnings; the 2 `firewall.rs` dead-code warnings are already on `main`), `cargo test --workspace`, `cargo build --workspace` all pass.

## Still owed

On Tunes@cast: pause for at least 60 s and confirm `consumer gone … ended_by_us=false` is followed by no advance and no unbidden play, then play restarts the same track at the paused position. This takes over Tunes, which Joel is using through his daemon on j15r, so it waits for his go-ahead, or for a rebuild of his daemon and his `canon player`.
