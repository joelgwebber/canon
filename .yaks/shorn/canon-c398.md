---
id: canon-c398
title: Extract toque into its own repo, with a settle hook and a wait action
type: task
priority: 3
created: '2026-09-27T17:40:42Z'
updated: '2026-09-27T18:09:57Z'
parent: canon-3db9
labels:
- tui
---

Side-quest for the TUI (Joel chose this over a git dep on yaks or a path dep, 2026-09-27). Split crates/toque out of ~/src/rs/yaks with its history into ~/src/rs/toque; add HeadlessApp::settle (called before each frame, so an app with async state, like canon's WebSocket client, can absorb what its keys caused) and a 'wait <ms>' action (let time pass, then settle), for watching playback. Creating/pushing the GitHub repo is Joel's call; yaks switching to the new repo is a follow-up there.

---
▸ 2026-09-27T17:51:59Z [Joel Webber]
Done locally (2026-09-27): ~/src/rs/toque holds crates/toque's history (4 commits) plus 'Stand toque up as its own repository', 'Let apps with state of their own settle before each frame' (settle + wait) and 'Read a wide glyph as one character in text frames' (the grid printed the blank continuation cell, so 東京 read as 東 京; yaks' snapshots with ⏳ may change when it adopts this). canon depends on it by path (../rs/toque). Left: create/push rocketsurgery-games/toque (Joel's call), switch canon to the git dep, and switch yaks off crates/toque.

---
▸ 2026-09-27T18:09:57Z [Joel Webber]
Published 2026-09-27 (Joel's go-ahead): github.com/rocketsurgery-games/toque, main at 792624f. canon now depends on it by git (Cargo.lock pins 792624f); canon-tui tests pass against it. Left: move yaks off crates/toque onto this repo (a yaks-farm task; the wide-glyph fix may change yaks snapshots containing ⏳).
