---
id: canon-118a
title: A track that fails to open stops the queue instead of moving on
type: bug
priority: 2
created: '2026-09-27T23:42:49Z'
updated: '2026-09-27T23:56:37Z'
verify: cargo test -p canon-core fail
---

Seen with canon-e828 (2026-09-27): one unplayable entry left the player in Error, and nothing played until Joel pressed next. EngineEvent::Failed only sets state=Error. With autoplay/queue playback running, a failed open (not a failed output) probably should skip to the successor, with some bound so a queue of all-unplayable entries doesn't spin.

---
▸ 2026-09-27T23:56:37Z [Joel Webber]
Fix: EngineEvent::Failed while the entry is still Loading (never started) skips to successor() with a warn ('skipping "<title>", which failed to start: <why>'); after MAX_SKIPS=5 consecutive failures it stops in Error as before. The count resets on Loaded and on any command. A failure after playback started keeps the old behaviour (Error). Tests: an_entry_that_fails_to_start_is_skipped, failures_in_a_row_stop_the_queue. Not exercised on the daemon: the control client resolves items before queueing (a bogus id is refused: 'not found: tidal /v1/tracks/1'), and making a real entry unplayable means signing a service out of the credentials Joel's daemon shares. The controller already sends Failed for a failed open; that path is unchanged.

---
▸ 2026-09-27T23:56:37Z [Joel Webber]
verify: `cargo test -p canon-core fail` -> PASS (exit 0)
