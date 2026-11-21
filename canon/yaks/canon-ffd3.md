---
id: canon-ffd3
title: Point canon at joelgwebber/toque
status: shaving
type: task
priority: 2
created: '2026-11-21T04:22:25Z'
updated: '2026-11-21T04:54:09Z'
tags:
- tui
verify: cargo test -p canon-tui
---

canon depends on `github.com/rocketsurgery-games/toque` (locked at 72ac14c); the maintained repo is `github.com/joelgwebber/toque`, where shared TUI facilities are now being extracted (toque farm, yak-2107). Joel: move it over.

Touches `Cargo.toml`, `Cargo.lock` (`cargo update -p toque`), `docs/tui.md` and `AGENTS.md` (the link, and the `../rs/toque` clone path).

#### 2026-11-21T04:51:39Z

*tui-design · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T04:54:09Z

*tui-design · kind: verify · result: pass*

verify: `cargo test -p canon-tui` -> PASS (exit 0)
