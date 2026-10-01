---
id: canon-ae6e
title: Favorites import stops re-saving locally-unsaved items
type: task
priority: 3
created: '2026-10-01T01:22:08Z'
updated: '2026-10-01T01:22:08Z'
parent: canon-65f7
depends_on:
- canon-f917
labels:
- library
---

Known bug found during the playlist design review: a re-import of favorites re-saves tracks/albums/artists the user has since unsaved locally. Favorites import should only add what's newly favorited upstream since the last import, never resurrect something the user removed locally.
