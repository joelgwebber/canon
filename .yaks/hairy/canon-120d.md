---
id: canon-120d
title: 'Playlist history: version on every edit, list, restore'
type: task
priority: 2
created: '2026-10-01T01:21:34Z'
updated: '2026-10-01T01:21:34Z'
parent: canon-65f7
depends_on:
- canon-f917
labels:
- library
---

A version is recorded whenever a local playlist's track list changes (add/remove/move/merge/create). List a playlist's versions and restore one. Comes before merge so a bad merge is always undoable. Keep it simple: snapshot the ordered track list per version, no diffing yet (diff is deferred).
