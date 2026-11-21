---
id: canon-e536
title: 'TUI: pane subjects replace the page stacks'
status: shorn
type: task
priority: 2
created: '2026-11-21T16:17:15Z'
updated: '2026-11-21T16:38:42Z'
parent: canon-032a
depends_on:
- canon-2797
tags:
- tui
---

Design §The pane and its subjects: Enter on an album, artist or playlist shows it in the pane (the list stays put); pane focus with Enter in, Esc out; actions act on the focused side. Replaces Tab-scoped page stacks. Narrow: the pane takes the body while focused.

#### 2026-11-21T16:28:17Z

*tui-design · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T16:38:42Z

*tui-design*

Landed.

- `App`: `lists: [Option<Page>; 3]` (each view's list) and `pane: Vec<Page>` (subjects, opened from the list replacing, from the pane stacking) plus `focus: Focus`. `page()` is the page with the keys, so every existing action (enqueue, save, copy, merge, rename, delete, remove, move) acts on the focused side unchanged.
- `back()`: pane with more than one subject pops; pane with one hands focus to the list (subject stays on show); list clears the pane. Queue's `Esc` clears it too.
- Reload after a playlist change reads both the list and the pane's subject.
- Render: `render_view` (left) and `render_pane` (right, or the whole body when narrow and focused); a page's cursor shows only when focused; the prompt types over the list's heading only; pane track rows drop the album column.
- Keys: `Enter` opens, `Esc`/`Backspace` back; `h`/`l` unbound (design: they never move between list and pane).

Evidence: new test `an_album_opens_in_the_pane_beside_its_list` (focus, list cursor kept, wide vs narrow frames, Enter plays, Esc layers, re-open replaces); existing browsing tests pass on the new model (one updated: a removal now reloads both sides); new doc frame `tui-pane.svg`, looked at as PNG and the pane's columns fixed from it; headless run against a test daemon on 7399 with the real library (`2 ] Enter j 1 2 Esc Esc`): album in the pane beside Albums (229), pane kept across a view switch, Esc layers. Green bar.

Follow-up noticed: the `?` popup falls back to one column whenever a help line is too long for half the width (it happened twice this phase). Worth a toque `popup_lines` improvement: wrap or cap the name column rather than lose the second column.

#### 2026-11-21T16:38:42Z

*tui-design · kind: move · from: shaving · to: shorn · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: shaving -> shorn
