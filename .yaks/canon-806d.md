---
id: canon-806d
title: 'Executor role: effects down, engine events up, over the WS'
status: shaving
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T20:48:43Z'
parent: canon-88a7
depends_on:
- canon-e329
- canon-973d
source: arch
---

Today `Effect` and `EngineEvent` travel over an in-process mpsc between the actor and `PlaybackController`. Make them serialisable protocol messages and add an executor role to the WebSocket: an executor connection receives its player's effects and sends back generation-tagged engine events, renderer reports, and its discovered outputs (`SinkInfo`). The actor keeps judging staleness by generation exactly as now; the network adds latency, not new semantics.

Also: an executor disconnecting puts its actor into a disconnected/paused state (no fail-back to local; there is no local on the server).

Parent decisions: canon-88a7.

---
▸ 2026-10-17T20:48:43Z [delta:The Great Decoupling]

moved: hairy -> shaving
