---
id: canon-fdf3
title: Move the queue into the player actor; seq on queue changes; expose queue contents
type: task
priority: 2
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T14:51:02Z'
labels:
- arch
- state
- api
---

From canon-ba30 (G). The queue (queue/index/active) lives in PlaybackController behind a Mutex and is merged into a second snapshot stream by run_snapshot_publisher. Consequences: queue changes do not bump seq, which breaks the contract that a new seq means something happened (enqueue while playing only nudges dirty). There are two snapshot streams, and main.rs log_snapshots logs the one without the queue. QueueView is only {len, index}, so no client (TUI, MCP, mobile) can render the queue. Direction: queue state becomes actor state, the queue Commands are real transitions there, and the controller is a pure effect executor (resolve, engine, sink) driven by what the actor decided. One snapshot stream. Also stop counting every command as a transition (Play while Playing currently bumps seq). Watch the generation/auto-advance race semantics; they must survive the move. Before canon-c67f: MCP tools will want queue contents.
