---
id: canon-940b
title: Pause or play locally when a renderer is lost?
status: hairy
type: task
priority: 2
created: '2026-10-04T16:24:10Z'
updated: '2026-10-04T16:24:17Z'
parent: canon-5303
labels:
- sink
needs: human
---

---
▸ 2026-10-04T16:24:17Z [Joel Webber]
When a renderer is lost for good (reconnect gave up), the player fails back to local and restarts there (canon-08a9's 'never wedged'). On a box nobody is listening at, that is the playhead running on into silence (canon-5303); on a box with no audio device at all, local fails and the queue skips up to 5 entries before it errors. Recommendation: pause at the listener's position on the lost output's behalf (sink becomes local, state paused, sink_lost says why), so pressing play resumes here and choosing an output resumes there. Joel: pause, or keep playing locally?
