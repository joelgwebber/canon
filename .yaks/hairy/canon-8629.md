---
id: canon-8629
title: Lock-free ring buffer + realtime-safe output callback
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T02:01:38Z'
parent: canon-b192
labels:
- audio
---

One SPSC ring (rtrb/ringbuf) of interleaved frames between decode and output, sized in TIME (~200-500ms) not chunk count. Producer parks on full (backpressure); the callback NEVER blocks, allocates, locks, or syscalls. On underrun: emit silence but keep advancing the frame counter so position doesn't freeze on a hiccup (tideway's good instinct, kept). No GIL means the buffer can be genuinely small.
