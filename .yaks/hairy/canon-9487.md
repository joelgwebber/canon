---
id: canon-9487
title: Snapshot + seq + delta stream to clients
type: task
priority: 2
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T02:01:37Z'
parent: canon-e284
labels:
- state
- api
---

Clients get an immediate full snapshot on connect, then deltas; reconcile by seq; terminal states (ended/error) always forwarded. Generalizes tideway's reset-then-delta download broker to player + queue. Prefer emitting (position, timestamp, rate) so clients interpolate smoothly without a 4Hz server poll.
