---
id: canon-973d
title: One player actor per player id on the server
status: hairy
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T19:21:43Z'
parent: canon-88a7
source: arch
needs: human
---

`run_serve` spawns exactly one `PlayerHandle`, and nothing in `Command` or `PlayerSnapshot` says which player it means.

The server holds one actor (queue, generations, snapshot) per player id, created on first sight and kept across disconnects, so a player reconnecting with the same id resumes its own queue. A controller connection names the player it drives; two players for one user must work. Autoplay runs per actor. Persisting queues across a server restart: decide here, record it.

Parent decisions: canon-88a7 (Cut A).

---
▸ 2026-10-17T19:21:43Z [delta:The Great Decoupling]

asked: needs human

Should each player's queue (and position) survive a server restart? My lean: yes. Today a restart loses the one queue, which is tolerable on one laptop. With a server that phones and the box share, a restart (deploy, reboot) silently wiping every player's queue would be the norm, and the library sqlite is already the store. Cost: a schema migration (live-check with its own --state-dir, per AGENTS.md) and deciding when a queue is written (every change vs. debounced). Without persistence, a reconnecting player gets an empty actor after a restart.
