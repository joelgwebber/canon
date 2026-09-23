---
id: canon-edc7
title: 'Queue management: play next, enqueue many, replace-at, jump, remove, move, shuffle, repeat'
type: task
priority: 2
created: '2026-09-23T21:38:54Z'
updated: '2026-09-23T21:50:10Z'
parent: canon-23f5
labels:
- state
- api
verify: cargo test -p canon-core player && cargo test -p canon-daemon control
---

Core queue verbs in the player actor, on the API, and in canon control. Editing the upcoming entry after it was prepared for a gapless join must never play the wrong track: the preparation is superseded and dropped (Effect::Unprepare), and if the engine joined it anyway the actor restarts onto the right successor at the boundary.

---
▸ 2026-09-23T21:50:03Z [Joel Webber]
Built: Command::{EnqueueMany, PlayNext, Replace{tracks,start}, Jump, Remove, Move, Shuffle, SetRepeat}; Repeat {off,all,one} on QueueView; successor()/following() honour repeat; edit_queue tracks current+prepared through edits; a prepared entry that is no longer the successor is superseded (Effect::Unprepare -> AudioPlayer::cancel_next, with a cancelled marker for one still opening); a superseded entry joined anyway (Advanced, or a flow Joined crossing) starts the real successor. API: queue_add{items,at,start} (items expanded by the library: entity or service id, albums as tracklists), jump/remove/move/shuffle/repeat. canon control: play|add|playnext <items>, jump/rm/mv (1-based), shuffle, repeat. 9 new actor tests.

---
▸ 2026-09-23T21:50:03Z [Joel Webber]
On metal 2026-09-23, local output: play 55391795 55391796; add 33348478; playnext 520285418 -> [Brain Damage, Backstabber, Eclipse, Army of Me]; mv 4 2 -> [BD, Army, Backstabber, Eclipse]; rm 3 -> [BD, Army, Eclipse]; seek 3:30, (Army prepared at ~3:35), playnext 55391796 at ~3:39 -> [BD, Eclipse, Army, Eclipse]; BD played to 3:37+ then "playing 0:00/2:07 Eclipse [2/4]" with no loading between: the stale preparation was cancelled and Eclipse re-prepared and joined gaplessly. jump 3 -> Army; repeat one shown in status. The Tunes flow-mode run could not happen: LAN discovery went dark (canon-0109); that path is covered by a_superseded_flow_join_restarts_onto_the_real_successor and owed on metal.

---
▸ 2026-09-23T21:50:10Z [Joel Webber]
verify: `cargo test -p canon-core player && cargo test -p canon-daemon control` -> PASS (exit 0)
