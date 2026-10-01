---
id: canon-2175
title: Rename a local playlist from the TUI
type: task
priority: 2
created: '2026-10-01T13:08:23Z'
updated: '2026-10-01T13:08:23Z'
parent: canon-9c5d
depends_on:
- canon-4c6e
labels:
- tui
---

playlist_rename already exists as an op (and in canon control as 'pl rename'); nothing in the TUI calls it. Wire a key on a local playlist row to the prompt from canon-4c6e, pre-filled with the current name, and send playlist_rename on Enter.
