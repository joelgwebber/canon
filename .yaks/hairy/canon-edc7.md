---
id: canon-edc7
title: 'Queue management: play next, enqueue many, replace-at, jump, remove, move, shuffle, repeat'
type: task
priority: 2
created: '2026-09-23T21:38:54Z'
updated: '2026-09-23T21:38:54Z'
parent: canon-23f5
labels:
- state
- api
---

Core queue verbs in the player actor, on the API, and in canon control. Editing the upcoming entry after it was prepared for a gapless join must never play the wrong track: the preparation is superseded and dropped (Effect::Unprepare), and if the engine joined it anyway the actor restarts onto the right successor at the boundary.
