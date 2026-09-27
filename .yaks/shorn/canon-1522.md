---
id: canon-1522
title: 'TUI phase 1: typed protocol both ways, connection, now playing, queue and transport keys, headless mode'
type: task
priority: 3
created: '2026-09-27T17:42:31Z'
updated: '2026-09-27T17:51:59Z'
parent: canon-3db9
labels:
- tui
verify: cargo test -p canon-tui && cargo test -p canon-api --lib
---

Protocol types serialize and deserialize in both directions (round-trip tested). canon-tui crate: pure App (apply server messages, keys to requests, render), a WebSocket connection task, the live terminal loop, and canon tui --headless via toque (settle waits for the replies a key asked for; wait lets playback move). Now-playing bar (state, title/artist, progress interpolated from rate, playing_from, output, volume), queue list with jump/remove/move, transport keys.

![tui-live-frame](artifacts/canon-1522/tui-live-frame.txt)

![tui-transport-session](artifacts/canon-1522/tui-transport-session.txt)

---
▸ 2026-09-27T17:51:53Z [Joel Webber]
Live (test daemon :7399, local output at vol 0, Prog queued, 2026-09-27): canon tui --headless showed the queue (14 entries, ▶ on the playing one), now playing with album, progress interpolated between snapshots (0:01→0:04 across a wait 2000), 'tidal flac 16/44.1', local · connected, and the key overlay. Keys, each confirmed by the next settled frame's state header or the daemon's reply: n (queue=0→1, state loading→playing), Right (0:03→0:12), Space/Space (paused, position held; then playing), J (entries 4/5 swapped), d (queue 14→13), r r (repeat all→one→off), - at 0% (stays). Live terminal mode smoke-tested under script(1) at 100x20: draws, j moves, q quits and restores the screen. 10 unit/insta tests on scripted server messages; protocol round-trip tests in canon-api.

---
▸ 2026-09-27T17:51:59Z [Joel Webber]
verify: `cargo test -p canon-tui && cargo test -p canon-api --lib` -> PASS (exit 0)
