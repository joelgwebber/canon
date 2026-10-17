---
id: canon-5ce4
title: Local-output position as reports, not a shared FrameClock
status: hairy
type: task
priority: 2
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T19:15:46Z'
parent: canon-88a7
source: arch
---

`PlayerHandle::clock()` hands the engine an `Arc<FrameClock>` that the actor reads: the one place executor and actor share memory instead of messages. Across the decoupling seam that can't hold. Local-output position must reach the actor as periodic reports, as renderer position already does, and `position.rs` gains that as a third authority case (interpolate between reports; transitions still carry `seq`).

Touches position, so it is not shorn on unit tests: check seek/pause/resume drift on real local output and on Tunes (AGENTS.md, "Verifying against real hardware").
