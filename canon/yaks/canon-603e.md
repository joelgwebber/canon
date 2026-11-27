---
id: canon-603e
title: Rethink tab navigation
status: hairy
type: idea
priority: 3
created: '2026-11-21T19:06:51Z'
updated: '2026-11-27T00:26:01Z'
parent: canon-44f1
tags:
- tui
---

In the current design, we have numbered tabs at the top, some of which have nested tab-like structured -- eg, Library's "tracks - albums - artists" and Playlists' "mine - tidal - ...".

We have a mixture of keyboard affordances for these, with the top-level controlled by [shift-]tab and 123..., and the secondary controlled by [/] keys. The secondary level can be a bit hard to follow, because the current selection is rendered at the front of the list, rather than with explicit highlighting.

Let's reconsider this stacked-tab design holistically, against the other yaks in this family, to see how we can best reduce extra states and confusion.
