---
id: canon-4c6e
title: A reusable text-prompt widget in the TUI
type: task
priority: 2
created: '2026-10-01T13:08:15Z'
updated: '2026-10-01T13:08:15Z'
parent: canon-9c5d
labels:
- tui
---

self.input: Option<String> (app.rs) exists only for the search box -- search_key() hardcodes Enter as 'run a search'. Renaming a playlist and naming a duplicate (both below) each need the same keystroke-collection UI for a different purpose.

Generalize it: carry what the prompt is for (search / rename / name-a-copy / ...) alongside the string, pre-fillable (rename wants to start from the current name), and dispatch on Enter by that tag instead of assuming search. Keep it the one text-entry mechanism in the app rather than growing a second parallel one.
