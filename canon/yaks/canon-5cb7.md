---
id: canon-5cb7
title: 'TUI: outputs picker overlay and settings as a view'
status: shaving
type: task
priority: 2
created: '2026-11-21T16:17:15Z'
updated: '2026-11-21T16:27:00Z'
parent: canon-032a
tags:
- tui
---

Decisions 4 and 6: Outputs leave the tab row for a picker overlay (Ctrl-O); Settings and sign-in stay a view, reached by key rather than Tab cycling. Tabs become Queue, Library, Playlists, Search.

#### 2026-11-21T16:19:45Z

*tui-design · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T16:27:00Z

*tui-design*

Landed.

- `Tab` is Queue, Library, Playlists, Search (`1`-`4`, `Tab`), plus Settings on `,`, named on the tab row only while open.
- `Ctrl-O` opens the outputs picker (modal `Ctx::Outputs`): cursor on the output playing now (re-placed when the sink list arrives), `Enter` plays there and closes, `f` flips the mode and stays, `Esc`/`q`/`Ctrl-O` close. Drawn over the list, so the pane stays whole.
- State header: `picker=outputs:<cursor>` while open.

Evidence: tests (the outputs test now goes through Ctrl-O, asserts modal keys and close-on-choose), snapshots reviewed one by one (tab row, popup, picker), doc frames regenerated and the outputs and settings frames looked at as PNGs; headless run against a test daemon on 7399 with the real eight outputs (`C-o j Esc , Tab`): picker listed them, Esc closed, `,` opened Settings, Tab went on to Library. Green bar.
