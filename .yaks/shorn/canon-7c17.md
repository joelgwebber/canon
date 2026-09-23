---
id: canon-7c17
title: Renderer reports that predate a transport command flip the state back
type: bug
priority: 2
created: '2026-09-23T19:42:56Z'
updated: '2026-09-23T19:45:52Z'
parent: canon-7718
labels:
- arch
- sink
- state
verify: cargo test -p canon-core --lib player
---

Found verifying canon-fdf3 on Cast (intermittent; a rerun was clean). Pause at t, and the Cast poll just before the device processes our PAUSE still reports Playing, so the actor goes paused -> playing (seq bump), then paused again when the device confirms. Play does the same with a stale Paused. Observed: (10 paused) (11 playing) (12 paused) ... (13 playing) (14 paused) (15 playing). The window is one poll interval (500ms Cast, 500ms DLNA), and it existed before canon-fdf3 (it now occurs with different timing).

Fix: in the actor, after a transport command on a renderer stream, expect the matching condition (Pause: Paused; Play: Playing or Buffering). Until the device confirms, or a short window (about 3s) passes, a contradicting RendererState is taken to predate the command and dropped. After the window the device is the authority again, so a device that really ignored the command gets its say. That keeps the conditions-vs-edges rule intact: conditions keep flowing; only the ones we have reason to think are older than our command are discounted. Unit-test with paused time (tokio test-util). Verify on metal with repeated pause/play.

---
▸ 2026-09-23T19:45:51Z [Joel Webber]
Fixed in the actor. After Pause/Play on a renderer stream it expects Paused / (Playing|Buffering), and for CONFIRM_WINDOW (3s) a contradicting RendererState is taken to predate the command and dropped. A confirming report or the window lapsing ends the expectation, after which the device is the authority again. The expectation is cleared on every start/halt. Tests (paused tokio time): a_report_older_than_a_pause_does_not_undo_it, a_report_older_than_a_play_does_not_undo_it, a_device_that_ignored_the_pause_is_believed_after_the_window.

On metal, 4 pause/play cycles 3s apart on Tunes over Cast: exactly one transition per command, position frozen through each pause and resumed from the same ms:
  (5 playing ->7063) (6 paused 7081) (7 playing 7081->9758) (8 paused 9871) (9 playing 9871->12682) (10 paused 12798) (11 playing 12798->15713) (12 paused 15834) (13 playing 15834->18641)
The same on DLNA (Tunes@dlna), 2 cycles: (6 paused 7889) (7 playing 7889->10739) (8 paused 10754) (9 playing 10754->13713) (10 idle).
Before: (10 paused) (11 playing) (12 paused) (13 playing) (14 paused) (15 playing) for one pause/play pair.

---
▸ 2026-09-23T19:45:51Z [Joel Webber]
verify: `cargo test -p canon-core --lib player` -> PASS (exit 0)
