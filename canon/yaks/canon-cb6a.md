---
id: canon-cb6a
title: Work out list/detail design and affordances
status: hairy
type: idea
priority: 3
created: '2026-11-21T19:42:43Z'
updated: '2026-11-21T19:52:38Z'
parent: canon-3c5e
tags:
- tui
---

The current structure's a little odd, and could use a coherence pass:
- `Enter` is *both* "play from here" and "open in pane", making it impossible to display a track's details without playing it immediately.
- The detail pane is unaffected by list selection, leading to weird states. Eg:
	- Select a playlist, and the detail shows its tracks. Switch to queue, library, et al and it still displays the playlist's track list, but there's no way to get back to it.
	- `Esc` does close whatever was in the detail panel, taking you back to the default "now playing".
- Now playing can't be selected, which is probably fine-ish? But it would be nice to
