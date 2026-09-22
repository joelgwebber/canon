---
id: canon-8629
title: Lock-free ring buffer + realtime-safe output callback
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T15:38:34Z'
parent: canon-b192
labels:
- audio
verify: cargo test -p canon-audio
---

One SPSC ring (rtrb/ringbuf) of interleaved frames between decode and output, sized in TIME (~200-500ms) not chunk count. Producer parks on full (backpressure); the callback NEVER blocks, allocates, locks, or syscalls. On underrun: emit silence but keep advancing the frame counter so position doesn't freeze on a hiccup (tideway's good instinct, kept). No GIL means the buffer can be genuinely small.

---
▸ 2026-09-22T15:38:34Z [Joel Webber]
verify: `cargo test -p canon-audio` -> PASS (exit 0)

---
▸ 2026-09-22T15:38:34Z [Joel Webber]
Shorn: rtrb SPSC ring sized in time (~500ms) between decode thread and cpal callback; callback pops+pads-with-silence and advances FrameClock only by frames actually delivered (hiccup pauses, never freezes); producer parks on full (backpressure). Proven end-to-end by canon play-file (132300 frames, clean drain). Verify: cargo test -p canon-audio.
