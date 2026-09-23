---
id: canon-a0cb
title: 'Play after stop does nothing: Idle ignores Play even with a queue'
type: bug
priority: 2
created: '2026-09-23T20:57:27Z'
updated: '2026-09-23T20:57:27Z'
parent: canon-23f5
labels:
- state
- api
---

Found on metal (canon-3a6c run): stop, then play -> ack, no transition. player.rs Command::Play returns Transition::No unless Paused. After stop the queue and index survive (queue len 1, index 0), so Play from Idle with a non-empty queue should Start the current entry at 0 (and from Idle at the end of the queue, arguably restart the last entry or no-op). Every remote client's play button hits this.
