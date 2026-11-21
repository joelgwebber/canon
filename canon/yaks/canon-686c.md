---
id: canon-686c
title: Design a better player UI for the TUI
status: shaving
type: task
priority: 2
created: '2026-11-21T04:20:50Z'
updated: '2026-11-21T04:22:26Z'
tags:
- tui
needs: human
---

Iterate on a design dok for the next canon TUI: a real player UI, borrowing the views / pane / history / command-table model from the yaks TUI, with the reusable pieces (theme surfaces, keymap, images) coming from toque.

The dok is the deliverable; implementation yaks get filed from it once the open questions are settled.

#### 2026-11-21T04:21:49Z

*tui-design · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T04:22:26Z

*tui-design*

First draft of the design: [The canon TUI, second pass: a player](/doks/design/tui-player.md).

Shared pieces are planned in the toque farm under yak-2107 (theme surfaces, command table, image pane, mouse hit map).

#### 2026-11-21T04:22:26Z

*tui-design · kind: ask · needs: human*

asked: needs human

Open questions from the [draft](/doks/design/tui-player.md#open-questions), with leans:

1. Album art: client fetches `artwork_url` itself, or the daemon serves cached, resized art? Lean: the daemon.
2. Tracks on `<` `>` and `n`/`N` for find (matching yaks), instead of today's `n`/`p`? Lean: yes.
3. Keep shuffle on `s`? Lean: yes.
4. Settings and sign-in: a fifth view behind a key, or an overlay? Lean: a view.
5. Selecting a queue entry: leave Now playing in the pane, or show the entry's album? Lean: leave Now playing.
6. Outputs as a picker overlay (`Ctrl-O`) rather than a tab: agreed?
