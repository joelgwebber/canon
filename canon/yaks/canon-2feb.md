---
id: canon-2feb
title: 'TUI phase 6: key pass and mouse'
status: shaving
type: task
priority: 3
created: '2026-11-21T04:48:22Z'
updated: '2026-11-21T20:09:40Z'
parent: canon-3c5e
depends_on:
- canon-1c6e
tags:
- tui
---

Design decisions 2 and 3, and the mouse (toque yak-f352).

#### 2026-11-21T19:55:26Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T20:03:07Z

*herd-player*

Claim (herd-player, done in the main thread). Design decisions 2 and 3, and the mouse.

## Keys

- `<` `>` previous / next track. `n` and `p` move off transport.
- `n` `N` next / previous match of the last find. `/` in Library or Playlists is "find in this list": it moves the cursor to the first row whose title or credit contains the text, and `n`/`N` step through the matches. In Search, `/` is a new query, as today. That keeps the design's "`/` filter this list" small: a find, not a filter. A filter would be a new list state, and canon-2b45 is rethinking search vs filters, so the filter waits for that.
- `.` in any view jumps to Now playing: the queue's cursor goes to the playing entry, and the pane goes back to Now playing.
- Exceptions: `d`/`x` (remove from a playlist) stays lowercase as in the queue, since one key, one intent beats the tier rule here. `f` in the outputs picker stays too: modal, and it changes how a speaker is driven, not saved data. The exception notes stop pointing at canon-2feb.

## Mouse

toque `HitMap`. Render records the tab labels, list rows (queue, list, pane), outputs-picker rows and the progress bar.

- Click a row: move the cursor there and give that side the keys. Double-click: `Enter`.
- Click a tab: go there. Click the progress bar: seek.
- Wheel over a list: move by 3. Wheel over the strip: volume.

The live terminal enables mouse capture.

## Evidence

- Tests for each.
- A headless run against a test daemon on 7399, with `click`/`dclick`/`scroll` actions and one rendered frame (`svg`).
- Green bar.

![key-pass-find](canon-2feb/key-pass-find.svg "Live: double-clicked an album open in the pane; / blue found 'So Blue'")
