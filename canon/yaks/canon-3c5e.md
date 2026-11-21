---
id: canon-3c5e
title: 'TUI second pass: a player'
status: shaving
type: feature
priority: 2
created: '2026-11-21T04:48:13Z'
updated: '2026-11-21T17:31:10Z'
tags:
- tui
---

Build the design in [The canon TUI, second pass: a player](/doks/design/tui-player.md), phase by phase. Each phase ships on its own, with headless frames and `docs/tui.md` moving with it.

Phases 1 and 2 wait on toque's extractions (toque farm: yak-4b3b theme surfaces, yak-7658 command table), and on canon-ffd3 pointing canon at that toque.

#### 2026-11-21T05:00:17Z

*tui-design · kind: move · from: hairy · to: shaving · child: canon-5781 · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T17:31:10Z

*herd-player*

Batch 1 (herd-player), base canon `7b7ff8c`, both on claude-sonnet-5-5 (mid tier):

- `hist-1` → canon-1c6e (one history), thread `ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHXRoZIBxBCA0aLJyrhJPreUvVwLftHG`, test daemon 7398/7348.
- `art-1` → canon-5a2c (daemon album art), thread `ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHXTVpIBxBCA0aLJyrhJPreUvVwLftHG`, test daemon 7397/7349.

Disjoint scopes (canon-tui + docs/tui.md vs canon-api + canon-daemon + docs/arch.md); the only shared file is Cargo.lock (art-1 only).
