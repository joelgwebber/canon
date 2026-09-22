---
id: canon-23f5
title: Server-owned queue (next/prev, auto-advance)
type: task
priority: 2
created: '2026-09-22T03:49:53Z'
updated: '2026-09-22T19:42:36Z'
parent: canon-1190
labels:
- api
verify: cargo test --workspace
---

The play queue lives in the daemon, not any client — collapsing tideway's three separate 'now playing' notions into one shared truth. Holds an ordered list of TrackRefs + current index; exposes enqueue/next/prev/clear/move over the control API; auto-advances on EngineEvent::Ended. Carved out of canon-b46b, which lands transport + schema + push-stream first. Blocked on nothing structural, but most valuable once a real Source can resolve tracks to enqueue.

---
▸ 2026-09-22T19:42:35Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-09-22T19:42:35Z [Joel Webber]
Shorn: server-owned queue in the PlaybackController. Load replaces the queue; Enqueue appends + auto-starts when idle; Next/Previous navigate; Clear stops; auto-advance on EngineEvent::Ended starts the next track (shares the play_index path Next exercises). Generation-guarded, off-lock resolve so a Next racing an auto-advance can't double-skip/install a stale stream; events funnel through one processor task. Verified live over ws: enqueue auto-start, second enqueue queues without interrupting, next->track2, prev->track1, next-past-end returns 'no next track', clear->idle. Exposed as ws ops enqueue/next/previous/clear. Follow-up worth noting: snapshot doesn't yet expose queue length/index (clients infer from the changing track).
