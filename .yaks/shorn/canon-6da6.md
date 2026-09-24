---
id: canon-6da6
title: Surface queue state to clients (snapshot + control display)
type: task
priority: 2
created: '2026-09-22T21:39:13Z'
updated: '2026-09-22T21:39:31Z'
labels:
- api
- state
verify: cargo test --workspace
---

Expose the server-owned queue in the client-facing snapshot: PlayerSnapshot gains an optional QueueView {len,index}; the controller merges player state + queue view into its own snapshot stream (nudged on queue-only changes). canon control shows [i/n]. Also quiets benign Symphonia probe/isomp4 log noise.

---
▸ 2026-09-22T21:39:31Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-09-22T21:39:31Z [Joel Webber]
Shorn: PlayerSnapshot.queue = Option<QueueView{len,index}>; the controller publishes a merged stream (player state + queue view), republished on player changes and nudged via a Notify on queue-only changes. canon control renders [i/n]. Verified live over ws: 2 enqueues -> {len:2,index:0}; next -> {len:2,index:1}; clear -> None. Also quieted the benign symphonia probe WARN + isomp4 INFO in the default log filter.
