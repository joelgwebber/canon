---
id: canon-2797
title: 'TUI: list beside a pane, Now playing at rest'
status: shaving
type: task
priority: 2
created: '2026-11-21T16:17:15Z'
updated: '2026-11-21T16:19:17Z'
parent: canon-032a
tags:
- tui
---

Design §Layout and §The pane: on a wide terminal the body splits into the list and a pane whose resting subject is Now playing (track, artists, album, source and format, output, up next). Narrow terminals keep today's single list. No focus or subjects yet (that is the subjects yak); the transport strip stays.

#### 2026-11-21T16:17:25Z

*tui-design · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T16:19:17Z

*tui-design*

Landed. The pane (render_now_playing) shows title, artists, album, state, source/format, output, and up to 8 entries up next; nothing-playing says so. Width: 35% of the body clamped to 34..=56, from 100 columns. Evidence: snapshot a_wide_terminal_shows_now_playing_beside_the_list (120x18) plus a narrow-terminal check; doc frames regenerated and the library frame looked at (rasterized PNG) before and after narrowing the pane; green bar.
