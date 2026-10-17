---
id: canon-5ce4
title: Local-output position as reports, not a shared FrameClock
status: shaving
type: task
priority: 2
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T22:01:38Z'
parent: canon-88a7
source: arch
needs: human
---

`PlayerHandle::clock()` hands the engine an `Arc<FrameClock>` that the actor reads: the one place executor and actor share memory instead of messages. Across the decoupling seam that can't hold. Local-output position must reach the actor as periodic reports, as renderer position already does, and `position.rs` gains that as a third authority case (interpolate between reports; transitions still carry `seq`).

Touches position, so it is not shorn on unit tests: check seek/pause/resume drift on real local output and on Tunes (AGENTS.md, "Verifying against real hardware").

---
▸ 2026-10-17T20:37:28Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T20:40:46Z [worker-5ce4]

Started by worker-5ce4 at Sat Oct 17 08:40:42 PM UTC 2026 (date -u). yaks path: /home/joel/src/canon/.delta/worktrees/5pr8ctnrtekd/canon/.yaks/canon-5ce4.md

---
▸ 2026-10-17T22:01:07Z [worker-5ce4]

Decision: local position is a THIRD authority case, `LocalClock` in canon-core/src/position.rs, not a mode of `RendererClock`. The two report sources are opposite where it matters: renderer reports are coarse (DLNA whole seconds), lag, are stream-relative (need `origin`), get eased both ways, and keep extrapolating when they stop (liveness there is StreamBroadcaster::consumers()). Engine reports are exact, current, already on the track timeline (the engine rebases its own count at a join), so LocalClock takes a report ahead of its estimate as-is, eases only small (<250ms) backward jitter (1/8 per report), and STOPS when reports stop moving. Rejected: a `RendererClock` flag (would put origin/slew-ahead/no-stall branches on every method of both); reusing RendererClock as-is (it would extrapolate through a stall, which loses the frame clock's whole point).

---
▸ 2026-10-17T22:01:07Z [worker-5ce4]

Decision: the engine reports every 250ms (LocalClock::REPORT_EVERY, matching the actor's POSITION_TICK) from a dedicated `canon-position` thread per local playback, whether or not position moved. The cpal callback only does `fetch_add` on an atomic (unchanged: no alloc, no lock, no syscall). Rejected: reporting from the feed thread (blocks for seconds on a slow source while the 500ms ring keeps the device playing, so reports would stall when output didn't, a false stall); reporting from the callback (sends allocate/lock); a tokio task (canon-audio only has tokio "sync", and no Cargo.toml edits). The same thread does the gapless crossing check (20ms poll) under one mutex with report sends (`Tracker`), so `Advanced` and the first report on the new track go out in order, and no old-track report can arrive after `Advanced`.

---
▸ 2026-10-17T22:01:07Z [worker-5ce4]

Decision: interpolation between reports is wall time from the last report, capped at LocalClock::MAX_EXTRAPOLATION (1s = 4 periods). A report that didn't move while playing marks the clock stalled; reports that stop arriving for >1s also count as stalled. Stalled => snapshot `rate` = 0 (state stays Playing: the listener paused nothing), position held. LocalPosition is never a transition: `seq` unchanged; the tick re-emits when position OR rate changed, so clients learn a stall within one tick. Rejected: a new PlaybackState for stall (wire/protocol change, and would be a transition 4x/s on a flapping device); leaving rate=1 during a stall (clients would interpolate a frozen position forward, i.e. "frozen-and-hidden").

---
▸ 2026-10-17T22:01:07Z [worker-5ce4]

Decision: transport rules in the actor. Seek/restart: Effect::Start drops `local`, so the old engine's stale reports are ignored (they carry the old generation anyway) and position reads 0 while loading, then `Loaded{start_ms}` opens LocalClock at the seek point. Pause: clock frozen; reports during pause re-anchor (the count is truth). Resume: forgets last report so the first unmoved report after resume isn't a false stall. Advanced: LocalClock::rebase() to 0; engine's rebased reports follow. DeviceChanged: nothing to do (the engine's count runs on across a reopen; the reopen gap shows briefly as stalled), still a transition/re-emit as before. Rejected: keeping the old FrameClock epoch (no reader needs it once nothing outside the engine reads frames).

---
▸ 2026-10-17T22:01:07Z [worker-5ce4]

Decision: FrameClock moved out of canon-core into canon-audio (crate-private `canon-audio/src/clock.rs`), minus `epoch`/`seek`/`mark_device_change`, which only the actor used. `AudioPlayer::start` lost its clock argument; `PlayerHandle::clock()` is gone. The network path no longer advances any clock (nothing read frames-fed as position there). In the daemon only the two `AudioPlayer::start` call sites changed (controller.rs one line, spotify-play one line), so canon-806d's merge stays easy. Rejected: keeping FrameClock public in canon-core "for the engine" (it would invite a reader across the seam again).

---
▸ 2026-10-17T22:01:07Z [worker-5ce4]

Decision: `canon play-file` now drives the real PlayerHandle actor through a minimal executor (crates/canon-daemon/src/play_file.rs), with `--times N` (gapless hand-off) and `--script` (stdin: sleep/seek/pause/play/vol/next/reopen/quit), printing snapshots. Needs no daemon and no state dir, so it can live-check local position without touching Joel's library (schema migration 10 not yet on his checkout). Rejected: a test-only harness (not runnable on hardware by the next person); running `canon serve` with an empty --state-dir (local output would work, but it would need Tidal for real tracks, and play-file is the documented harness).

---
▸ 2026-10-17T22:01:23Z [worker-5ce4]

Evidence: green bar (2026-10-17, this tree).
$ cargo fmt --all --check                 -> exit 0
$ cargo clippy --workspace --all-targets  -> exit 0, 0 warning/error lines
$ cargo test --workspace                  -> exit 0; summed: passed=411 failed=0 ignored=5
  (canon-core lib: "test result: ok. 94 passed; 0 failed")
$ cargo build --workspace                 -> "Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.00s"
New tests: position.rs a_local_clock_carries_position_between_reports, local_report_jitter_never_steps_position_backward_visibly, a_local_report_far_behind_is_taken_as_it_is, a_stalled_output_stops_the_local_clock, a_local_clock_stops_when_reports_stop_coming, pausing_the_local_clock_freezes_it_and_reports_while_paused_place_it, a_gapless_crossing_rebases_the_local_clock_onto_the_next_track; player.rs local_position_reports_refresh_without_a_transition, a_stalled_local_output_shows_as_position_stopping, pause_and_resume_hold_local_position, a_local_seek_lands_where_the_new_stream_starts, a_local_crossing_rebases_position_onto_the_next_track; canon-audio clock.rs frame_clock_math (ported from canon-core), rebasing_keeps_frames_past_the_boundary. Ported: device_change_reemits_with_continuous_position, sink_failure..., fails_mid_way..., frames_fed_are_not_position_on_a_renderer (now via LocalPosition events), network_output (now asserts no LocalPosition on the network path), recovery.rs (asserts via reports).
Device test on the real USB output:
$ CANON_TEST_AUDIO=$PWD/target/5ce4-tone40.flac cargo test -p canon-audio --test recovery -- --ignored
  test forced_reopen_keeps_playing ... ok
  test result: ok. 1 passed; 0 failed; ... finished in 3.77s

---
▸ 2026-10-17T22:01:23Z [worker-5ce4]

Evidence: live local output (USB Audio card 0, Linux), real actor + engine via play-file, no daemon, no state dir. Test files: ffmpeg sine tones at volume 0.05, target/5ce4-tone40.flac (40s), target/5ce4-tone8.flac (8s).
$ printf 'sleep 3\nseek +10\nsleep 3\npause\nsleep 3\nplay\nsleep 3\nseek -5\nsleep 2\nreopen\nsleep 3\nquit\n' | RUST_LOG=canon_core::player=trace ./target/debug/canon play-file target/5ce4-tone40.flac --script
  seq=2   playing     3.051s rate=1           (after sleep 3: ~3.0s wall)
  stream opened ... start_ms=13049            (seek +10 from 3.05)
  seq=4   playing    16.112s rate=1           (3s after seek: 13.05 -> 16.11)
  seq=5   paused     16.114s rate=0 ... held at 16.092s for 3s (the engine's real count)
  seq=6   playing    16.092s ... 19.093s      (3s after play: +3.0s, resumed where it was)
  stream opened ... start_ms=14071            (seek -5 from 19.09)
  seq=8 ... 16.145s; reopen -> seq=9 16.409 rate=1, 16.133 rate=0 (stall during reopen, visible), 16.924 rate=1 ... 18.939
  seq=10  idle
Drift, actor estimate vs engine report, all 67 non-stalled reports: reported-derived min=-32 max=314 mean=1.9 ms (Snapped 32, Slewed 35). Steady state is +15/+16ms ahead, -1..-4ms behind. The 314 is the first report after the reopen stall: estimate was held at 16133, device resumed -> snapped forward to truth. The +53ms right after every stream open is the device pulling its first buffers faster than realtime.
Gapless: $ printf 'sleep 22\nquit\n' | RUST_LOG=canon_core::player=debug ./target/debug/canon play-file target/5ce4-tone8.flac --times 3 --script
  seq=2 playing 7.564s entry=1/3 #1 -> seq=3 playing 0.036s entry=2/3 #2 ... 7.562s -> seq=4 playing 0.014s entry=3/3 #3
  (no loading between, one seq per crossing, position rebased to ~0).

---
▸ 2026-10-17T22:01:38Z [worker-5ce4]

asked: needs human

Blocked on the shared state dir (schema migration 10 in this tree; not on Joel's checkout). Code is committed with the yak still shaving. Hardware checks that REMAIN before shearing, all on a test daemon (--bind 127.0.0.1:7397 --lan-port 7349, stopped with `pgrep -f 'bind 127.0.0.1:739[7]' | xargs kill`) once the shared state dir is safe:
1. Local output via the daemon with real Tidal tracks: `printf 'enqueue 33348478\nenqueue 520285418\nsleep 20\nseek +30\nsleep 10\npause\nsleep 3\nplay\nsleep 10\nseek -20\nsleep 10\nquit\n' | canon control --connect 127.0.0.1:7397` with RUST_LOG=canon_core::player=trace; check positions track wall time across seek/pause/resume and `seq` doesn't bump on refreshes. Also a gapless crossing on Tidal (seek near the end of 33348478 with 520285418 queued).
2. Tunes (sink Tunes, and Tunes@dlna): the same script. The renderer path is not meant to have changed (RendererClock untouched; the network feed loop just stopped advancing an unread clock), but the brief requires it, and canon-806d will move the controller anyway.
3. Sink switch Tunes -> local and back mid-track (local clock replaced/dropped on each restart; no stale LocalPosition moving a renderer position).
4. canon tui --headless against the test daemon: progress bar holds during a stall (rate 0) rather than running on.
Already done on real local hardware without a daemon: see the Evidence notes (play-file: seek/pause/resume/reopen/gapless; mean drift 1.9ms; recovery test passes).
