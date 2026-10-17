---
id: canon-973d
title: One player actor per player id on the server
status: hairy
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T19:15:46Z'
parent: canon-88a7
source: arch
---

`run_serve` spawns exactly one `PlayerHandle`, and nothing in `Command` or `PlayerSnapshot` says which player it means.

The server holds one actor (queue, generations, snapshot) per player id, created on first sight and kept across disconnects, so a player reconnecting with the same id resumes its own queue. A controller connection names the player it drives; two players for one user must work. Autoplay runs per actor. Persisting queues across a server restart: decide here, record it.

Parent decisions: canon-88a7 (Cut A).
