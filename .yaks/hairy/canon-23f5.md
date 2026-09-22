---
id: canon-23f5
title: Server-owned queue (next/prev, auto-advance)
type: task
priority: 2
created: '2026-09-22T03:49:53Z'
updated: '2026-09-22T13:17:44Z'
parent: canon-1190
labels:
- api
---

The play queue lives in the daemon, not any client — collapsing tideway's three separate 'now playing' notions into one shared truth. Holds an ordered list of TrackRefs + current index; exposes enqueue/next/prev/clear/move over the control API; auto-advances on EngineEvent::Ended. Carved out of canon-b46b, which lands transport + schema + push-stream first. Blocked on nothing structural, but most valuable once a real Source can resolve tracks to enqueue.
