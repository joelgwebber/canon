---
id: canon-f20f
title: 'TUI: a title full of combining characters overflows its row'
status: hairy
type: task
priority: 3
created: '2026-11-21T19:54:16Z'
updated: '2026-11-21T19:54:16Z'
tags:
- tui
---

Seen in the Library album list (canon-ed4b's live frame): an album whose title is mostly combining marks (Zalgo-style) draws past its column and over the pane. It does the same on main before the art work. fit() measures with unicode-width, which counts combining marks as 0 but the terminal (and toque's SVG) may not; truncation should go by grapheme clusters and clamp what a cell can hold.
