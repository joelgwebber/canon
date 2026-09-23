---
id: canon-a0cb
title: 'Play after stop does nothing: Idle ignores Play even with a queue'
type: bug
priority: 2
created: '2026-09-23T20:57:27Z'
updated: '2026-09-23T21:40:36Z'
parent: canon-23f5
labels:
- state
- api
verify: cargo test -p canon-core play_with_nothing_in_play
---

Found on metal (canon-3a6c run): stop, then play -> ack, no transition. player.rs Command::Play returns Transition::No unless Paused. After stop the queue and index survive (queue len 1, index 0), so Play from Idle with a non-empty queue should Start the current entry at 0 (and from Idle at the end of the queue, arguably restart the last entry or no-op). Every remote client's play button hits this.

---
▸ 2026-09-23T21:40:35Z [Joel Webber]
Play from Idle/Error starts the current entry, from Ended starts the queue at 0, empty queue is a no-op. On metal 2026-09-23 (Tunes): enqueue Army of Me, playing 0->6095, stop -> (5, idle), play -> (6, loading) -> (8, playing 0->8337).

---
▸ 2026-09-23T21:40:36Z [Joel Webber]
verify: `cargo test -p canon-core play_with_nothing_in_play` -> PASS (exit 0)
