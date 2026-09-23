---
id: canon-7c17
title: Renderer reports that predate a transport command flip the state back
type: bug
priority: 2
created: '2026-09-23T19:42:56Z'
updated: '2026-09-23T19:42:56Z'
parent: canon-7718
labels:
- arch
- sink
- state
---

Found verifying canon-fdf3 on Cast (intermittent; a rerun was clean). Pause at t, and the Cast poll just before the device processes our PAUSE still reports Playing, so the actor goes paused -> playing (seq bump), then paused again when the device confirms. Play does the same with a stale Paused. Observed: (10 paused) (11 playing) (12 paused) ... (13 playing) (14 paused) (15 playing). The window is one poll interval (500ms Cast, 500ms DLNA), and it existed before canon-fdf3 (it now occurs with different timing).

Fix: in the actor, after a transport command on a renderer stream, expect the matching condition (Pause: Paused; Play: Playing or Buffering). Until the device confirms, or a short window (about 3s) passes, a contradicting RendererState is taken to predate the command and dropped. After the window the device is the authority again, so a device that really ignored the command gets its say. That keeps the conditions-vs-edges rule intact: conditions keep flowing; only the ones we have reason to think are older than our command are discounted. Unit-test with paused time (tokio test-util). Verify on metal with repeated pause/play.
