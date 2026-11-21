---
id: canon-5781
title: 'TUI phase 1: selection band from toque theme surfaces'
status: shaving
type: task
priority: 2
created: '2026-11-21T04:48:20Z'
updated: '2026-11-21T05:02:26Z'
parent: canon-3c5e
depends_on:
- canon-ffd3
tags:
- tui
---

No layout change. Replace the fixed Indexed(238) band with the terminal-derived surfaces from toque, so the TUI reads on light themes. Waits on toque yak-4b3b and on canon-ffd3.

#### 2026-11-21T05:00:17Z

*tui-design · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T05:02:26Z

*tui-design*

Done in the working tree, not committed: it needs toque's `theme` feature, which is committed in toque (`ed595ee`) but not pushed, so the git dependency can't resolve it yet. Verified against the local toque via `--config patch...path=`:

- `render.rs`: the cursor band is `toque::theme::selected()`, replacing `Indexed(238)` and the forced white foreground (rows keep their own colours).
- `runtime.rs`: `toque::theme::init("CANON_THEME")` right after `ratatui::init`, before the key reader thread starts (otherwise the reader could eat the OSC reply).
- New test `the_cursor_band_follows_the_terminal_theme`: dark grey 237 by default, a light grey on a light terminal.
- `docs/tui.md` documents the band and `CANON_THEME`; doc frames regenerated.

Green bar against the patched toque: fmt ok, tests pass (canon-tui 43), build ok; clippy only the old `firewall.rs` warnings. Waiting on: push of toque `main`, then `cargo update -p toque` and commit.
