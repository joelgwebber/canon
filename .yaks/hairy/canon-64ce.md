---
id: canon-64ce
title: Read renderer volume back into the snapshot
type: task
priority: 3
created: '2026-09-23T16:13:27Z'
updated: '2026-09-23T20:43:14Z'
parent: canon-7718
labels:
- sink
---

From canon-44f4. The device is the authority on its own volume, but the snapshot volume is whatever the player last set (1.0 on a fresh session), so a client shows 100% for a speaker at 0.3%. Cast: receiver status carries volume {level, muted}. Poll it or read the set_volume reply, and report it as a RendererEvent (a condition, so it keeps flowing) that the player folds into volume/muted. DLNA: RenderingControl GetVolume/GetMute or LastChange. Selecting a sink must keep not pushing the player volume to the device. Adopt the device level instead; pushing 1.0 would blast the room.
