---
id: canon-f1d2
title: 'TUI phase 2: keys through the toque command table'
status: shorn
type: task
priority: 2
created: '2026-11-21T04:48:20Z'
updated: '2026-11-21T05:42:30Z'
parent: canon-3c5e
depends_on:
- canon-5781
tags:
- tui
---

No key changes. Bindings move into a table; the ? reference, the hint line and the docs/tui.md tables are generated from it. Waits on toque yak-7658.

#### 2026-11-21T05:30:17Z

*tui-design · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T05:42:30Z

*tui-design*

Landed with toque `6a411f9` (its `keys` feature, extracted from yaks under toque yak-7658).

- [keys.rs](code:crates/canon-tui/src/keys.rs): `Cmd` (dotted names), `Ctx` (Global, Queue, Page, Setup), `BINDINGS`, the popup's sections, `load_user` (`<config>/canon/tui.yaml`).
- `App::handle_key`: Global first, then the tab's context, then `run_cmd`; the four `match`es are gone. Prompt and sign-in stay modal.
- The `?` popup is generated (`toque::keys::help_groups` + `popup_lines`), folded into columns.
- No key changes. canon's `VIM_ONLY` is `hjkl gG` (`d` stays: it removes, it's no half-page key).
- Rule tests: no shared keys, every command bound and named, labels parse, both families, case tier (with four listed exceptions, two flagged for canon-2feb), no tab binds a global key. Plus an override/family test through `handle_key`.

Evidence: green bar (fmt, clippy only the old firewall.rs warnings, all tests, build); a headless run against a test daemon on 7399 (Library: `j j ? Esc ] 1 j ?`) moved the cursor, switched the listing to albums and showed the generated popup per tab; doc frames regenerated, `tui-keys.svg` looked at.

#### 2026-11-21T05:42:30Z

*tui-design · kind: move · from: shaving · to: shorn · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: shaving -> shorn
