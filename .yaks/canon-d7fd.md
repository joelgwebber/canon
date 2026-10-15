---
id: canon-d7fd
title: Cast thread never drains rust_cast's message buffer, and never answers the receiver's PINGs
status: shaving
type: bug
priority: 2
created: '2026-10-04T16:24:17Z'
updated: '2026-10-04T16:51:14Z'
labels:
- cast
- sink
---

rust_cast's MessageManager pushes every frame a request isn't waiting for into message_buffer, which only receive() drains; canon's cast thread never calls receive(). So the receiver's own PINGs (every ~5s), the PONGs to our 2/s PINGs, and every unsolicited MEDIA_STATUS/RECEIVER_STATUS pile up for the life of the session: ~8k frames an hour, never freed. The module docs say they are drained; nothing does. It is also why the receiver's PINGs never get a PONG (canon-041e's untested hypothesis for Cast drops). Fix: after each poll, send our PING and receive() until its PONG, answering PINGs on the way; a PONG always arrives after everything buffered, so the buffer ends every cycle empty.

---
▸ 2026-10-04T16:51:14Z [Joel Webber]
Built 2026-10-04: cast.rs heartbeat() replaces the bare ping: ping, then receive() until our PONG, answering PINGs and logging a receiver CLOSE at info. On hardware (Library display, RUST_LOG=canon_sink::cast=trace, ~60s idle session): 117 cycles; the first read 5 frames (the launch's leftovers), then read=1 (just our PONG) on 115 cycles and read=2 once, i.e. the buffer ends every cycle empty, where before each cycle left at least that PONG behind. Still owed before shearing: the same on Tunes (the KEF is the speaker in canon-5303, and a receiver that never PONGs would stall the session here), during real playback.
