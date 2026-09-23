---
id: canon-3a6c
title: Stream watchdog fails an idle network output back to local
type: bug
priority: 2
created: '2026-09-23T20:52:50Z'
updated: '2026-09-23T20:52:50Z'
parent: canon-7718
labels:
- sink
- network
---

Found on metal while verifying canon-ba9d. The watchdog counts 'no consumers' whenever the newest stream isn't drained, including when nothing is loaded at all: a speaker selected with an empty queue, after stop, or after a track that failed to open. 20s later (10s startup grace + 10s absence) it logs 'renderer stopped consuming our stream' and fails back to local. Absence only means a takeover while a load is in flight (NetworkSession.current is Some); with nothing loaded the renderer has nothing to pull.
