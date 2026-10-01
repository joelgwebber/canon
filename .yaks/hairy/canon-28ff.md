---
id: canon-28ff
title: Refresh the current page after a playlist-mutating action
type: task
priority: 1
created: '2026-10-01T13:07:53Z'
updated: '2026-10-01T13:07:53Z'
parent: canon-9c5d
labels:
- tui
---

Copying or merging a playlist (canon-e55d) already leaves the visible listing stale -- the new or changed playlist doesn't show until the tab is left and re-entered, because Pending::Done just sets a notice (app.rs) and never re-runs the page's own request(). Rename and delete (canon-5fe5 and the rename yak) will have the same gap once built.

Fix it once, generically: after an op that changes what the current page is showing (copy, merge, and once they exist, create/rename/delete), re-issue that page's Source::request() in place -- the same request push_page already builds when opening a page fresh, just reused rather than re-pushing a new page onto the stack. No server change needed.
