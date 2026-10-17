---
id: canon-973d
title: One player actor per player id on the server
status: shorn
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T20:28:09Z'
parent: canon-88a7
source: arch
verify: cargo test -p canon-core restor && cargo test -p canon-library player_queue && cargo test -p canon-library merging_a_track_repoints && cargo test -p canon-daemon players && cargo test -p canon-api --test control_plane
---

`run_serve` spawns exactly one `PlayerHandle`, and nothing in `Command` or `PlayerSnapshot` says which player it means.

The server holds one actor (queue, generations, snapshot) per player id, created on first sight and kept across disconnects, so a player reconnecting with the same id resumes its own queue. A controller connection names the player it drives; two players for one user must work. Autoplay runs per actor. Persisting queues across a server restart: decide here, record it.

Parent decisions: canon-88a7 (Cut A).

---
▸ 2026-10-17T19:21:43Z [delta:The Great Decoupling]

asked: needs human

Should each player's queue (and position) survive a server restart? My lean: yes. Today a restart loses the one queue, which is tolerable on one laptop. With a server that phones and the box share, a restart (deploy, reboot) silently wiping every player's queue would be the norm, and the library sqlite is already the store. Cost: a schema migration (live-check with its own --state-dir, per AGENTS.md) and deciding when a queue is written (every change vs. debounced). Without persistence, a reconnecting player gets an empty actor after a restart.

---
▸ 2026-10-17T19:34:57Z [Joel Webber]

Yeah, I tend to agree. It's not that difficult, and players would be that much more stable.

---
▸ 2026-10-17T19:39:55Z [delta:The Great Decoupling]

picked up

---
▸ 2026-10-17T19:39:55Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T20:28:09Z [delta:The Great Decoupling]

verify: `cargo test -p canon-core restor && cargo test -p canon-library player_queue && cargo test -p canon-library merging_a_track_repoints && cargo test -p canon-daemon players && cargo test -p canon-api --test control_plane` -> PASS (exit 0)

---
▸ 2026-10-17T20:28:09Z [delta:The Great Decoupling]

Evidence (2026-10-17):
- Green bar: fmt clean; clippy --workspace --all-targets 0 warnings; test --workspace all ok; build ok.
- New tests: actor `Restore` (waits for play, resumes the saved entry at its position, forgets it once anything else starts, refused over a playing queue); store round trip + `merge_tracks` repointing saved queues; migration-path tests updated to drop the new tables; daemon `a_queue_outlives_the_server_that_held_it` (edits persist; a fresh registry restores repeat, index, tracks, Idle; another player starts empty); API `players_are_independent_and_shared_by_name` (volume on `kitchen` invisible to `default`; reconnecting as `kitchen` sees it; `?player=no%20spaces` refused at upgrade); TUI header names the player and shortens tab labels to fit.
- Live, own --state-dir (copy of the real library via `sqlite3 .backup`; schema migrates 9 -> 10, per AGENTS.md's own-port-is-not-own-database rule): serve on 7399/7347; `canon control --player kitchen`: add 520285418, add 33348478, repeat all -> 2 entries; default player's queue empty. sqlite: `kitchen|1|all|0` + 2 track rows. Restarted the daemon: log `restored its queue: 2 tracks, at 2 player=kitchen`; control --player kitchen showed both entries, idle, `(repeat all)`; default still empty; `canon tui --player kitchen --headless` showed `kitchen · local · connected` and the queue (this run exposed the header overlap, now fixed and tested).
- Not checked live: resuming mid-track after a restart. The isolated state dir has no streaming login (credentials aren't copied, per AGENTS.md), so nothing played; that path is covered by the actor test and `follow`'s unit test only. Worth one look on Joel's daemon after landing: play, restart, play.

Decisions:
- Players are named by `/ws?player=<id>` (1-64 of [A-Za-z0-9._-], default `default`), echoed in `hello`. Not an op: a connection drives one player for its life, which is what canon-806d's executor role needs. canon-65d2 replaces the self-asserted name with the token's identity.
- Persistence is in the library sqlite (migration 10: player_queues + player_queue_tracks, FK to tracks, repointed by merges). Tracks are written when the queue revision's ids change; entry/repeat whenever they move; position at most every 5s while playing, and on shutdown. Restored queues come back Idle (`Command::Restore`), never auto-playing: a server restart must not start sound in a room.
- The LAN stream server became a shared `StreamHost`, since every player's controller registers with the one `--lan-port`.

Known gaps (follow-ups, not regressions with one player): players are never forgotten (any name makes a lasting actor until auth caps it); two players can both play to this machine's local output and nothing stops two selecting one speaker, which goes away when executors move to players (canon-806d).

---
▸ 2026-10-17T20:28:09Z [delta:The Great Decoupling]

moved: shaving -> shorn
