---
id: canon-cf48
title: Sink trait + session lifecycle (RAII un-silence)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T02:01:38Z'
parent: canon-7718
labels:
- sink
---

trait Sink { start(track); pause/resume/stop/seek; set_volume; health signal }. LocalSink, CastSink, DlnaSink. The player mutes local by ROUTING, not a global fill(0) flag that leaked forever in tideway. Un-silencing / route-restore is tied to dropping a session guard (Drop), so a skipped teardown is structurally impossible. Teardown is driven by a liveness detector (TCP close, Cast status timeout, DLNA poll), not only by a discovery-remove event.
