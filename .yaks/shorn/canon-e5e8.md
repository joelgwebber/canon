---
id: canon-e5e8
title: 'Position clock: atomic frame counter + device epoch'
type: task
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T03:09:36Z'
parent: canon-e284
labels:
- state
- audio
---

The realtime callback owns NO clock-of-record. It publishes frames_played: AtomicU64 plus a device-epoch id. The control task derives position_ms from frames on a fixed timer and emits — so a stalled callback is VISIBLE (frames not advancing) instead of tideway's invisible frozen emitter. Epoch bump on device change lets position rebase correctly across a stream reopen.

---
▸ 2026-09-22T03:09:36Z [claude]
Delivered as canon-core FrameClock (state.rs): atomic frames+epoch+sample_rate. advance() from the realtime callback (later, canon-audio); position()/position_ms() derived; reset() rebases for a new track; seek() and mark_device_change() model discontinuities (seek moves the timeline+epoch; device change bumps epoch only, timeline continuous). Covered by the frame_clock_math test.
