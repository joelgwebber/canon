---
id: canon-efa9
title: A renderer dropping a stream we are still feeding skips the rest of the track
type: bug
priority: 2
created: '2026-09-27T22:09:22Z'
updated: '2026-09-27T22:09:22Z'
labels:
- cast
---

Seen during canon-9c73 (2026-09-27): Tunes@cast closed our HTTP stream mid-track (stream server: consumer gone ... ended_by_us=false) with our feed still running, then answered status polls with no entries. cast::media_gone reads 'media gone after it started' as RendererEvent::Ended, so the queue advanced ~2.5 minutes early. The stream had not been drained (StreamRoutes::is_drained false), so this was the renderer abandoning it, not the track ending. Probably: when media vanishes while our stream is not drained and the position is well short of the duration, restart the current track on the same renderer at the listener's position (like reconnect()), not Ended. canon-9c73 makes the drop much rarer (bigger renderer buffer) but doesn't handle it.
