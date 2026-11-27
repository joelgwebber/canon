---
id: canon-1d6c
title: Some affordance for undoing destructive operations on library/playlists
status: hairy
type: feature
priority: 2
created: '2026-11-21T18:11:05Z'
updated: '2026-11-27T00:26:05Z'
parent: canon-0022
tags:
- tui
---

I find myself accidentally hitting `d` all the time (to go down a page, because... vim-brain), and accidentally deleting a track from a playlist. While we're also going to address the confusing key choice, it would be really helpful if, at least for profoundly destructive operations like this, we had some basic 'undo' affordance.

Let's consider the design carefully before committing to one, though. "Generalized" undo can easily turn into a very complex problem that pervades the APIs and data model. While I'd be ok with that, if it fit our model comfortably, we should also remain open to simpler approaches that cover the most destructive cases.
