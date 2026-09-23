---
id: canon-bc84
title: Accurate cast position from MEDIA_STATUS (reconcile frames-fed vs reported)
type: task
priority: 3
created: '2026-09-23T00:24:18Z'
updated: '2026-09-23T00:24:18Z'
parent: canon-dde4
labels:
- sink
---

v1 drives position from frames fed to the encoder, which leads actual Cast playback by the Cast buffer (~1-3s). Reconcile the FrameClock to the Cast MEDIA_STATUS reported position (seek clock on each status, free-run by wall-time between). Needs a wall-time clock advance on the network path.
