---
id: canon-394c
title: canon player's logs draw over its TUI
status: shorn
type: bug
priority: 2
created: '2026-10-18T15:05:05Z'
updated: '2026-10-18T15:07:55Z'
parent: canon-88a7
source: arch
---

canon player with its TUI logs to stderr, the terminal the TUI draws on, at info by default; cast's 'load accepted' Debug dump (and anything else at info) lands on the screen. Found by Joel. A command that owns the terminal should log to a file.

---
▸ 2026-10-18T15:05:18Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-18T15:07:55Z [delta:The Great Decoupling]

Fixed. `canon tui` and `canon player` with their full-screen TUI log to `<state-dir>/logs/canon.log` (appended, no ANSI); everything else still logs to stderr (never stdout). `cast load accepted` no longer dumps the whole Cast status at info (that moved to debug).

Evidence: the player with its real TUI under a pseudo-terminal (python pty) against a test server, a second client sending `sink Tunes@cast`, `enqueue 33348478`, `stop`: 4747 bytes drawn, 0 log lines among them; the log file holds `carrying out this player's playback`, `stream server on …:7347`, `cast load accepted`, consumer connected/gone. Green bar: fmt, clippy 0, test --workspace, build.

Decision: a file in the state dir rather than suppressing logs or raising the TUI's filter to warn: warnings are exactly what someone debugging a player wants, and RUST_LOG still works, into the file. Rejected: an in-TUI log pane (more UI than this needs now).

---
▸ 2026-10-18T15:07:55Z [delta:The Great Decoupling]

moved: shaving -> shorn
