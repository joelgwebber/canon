---
id: canon-0205
title: Simultaneous local + network output (multi-room) via OutputRoute/LocalGate
type: task
priority: 3
created: '2026-09-23T00:24:18Z'
updated: '2026-09-23T00:24:18Z'
labels:
- sink
---

The cf48 OutputRoute/LocalGate/RouteGuard RAII primitive is built for this: play to local AND >=1 network sinks at once, muting/routing local by the gate (cpal callback consults LocalGate), teardown RAII-bound. v1 Cast uses single-output restart instead, so this is where the gate finally gets wired into the hot path. Depends on canon-dde4.
