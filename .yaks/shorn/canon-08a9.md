---
id: canon-08a9
title: Device/sink changes as first-class state transitions
type: task
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T03:09:36Z'
parent: canon-e284
labels:
- state
- sink
---

A DeviceLost / DeviceChanged / SinkFailed event is a real transition that re-emits, never an out-of-band stream swap on a side thread (the exact shape of tide-2f85). 'Which sink is active' is STATE, not a side effect. Recovery paths reconcile and emit; they can't leave the emitted view stale while audio continues.

---
▸ 2026-09-22T03:09:36Z [claude]
Delivered inside the actor: EngineEvent::DeviceChanged marks a clock discontinuity and re-emits with CONTINUOUS position (the tide-2f85 fix, not a reset/freeze); EngineEvent::SinkFailed on the active sink fails back to the local sink without wedging playback. Both are first-class transitions through the one actor. Covered by device_change_reemits_with_continuous_position and sink_failure_falls_back_to_local tests.
