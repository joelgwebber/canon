---
id: canon-4b3b
title: Merge a source into a local playlist, append-only by recording identity
type: task
priority: 2
created: '2026-10-01T01:21:45Z'
updated: '2026-10-01T01:21:45Z'
parent: canon-65f7
depends_on:
- canon-5b2b
- canon-120d
labels:
- library
---

playlist_add gets a merge mode: given a source (local or remote, via the canon-5b2b facade), append only the tracks whose recording isn't already in the target playlist (match by the same recording-identity canon already uses for cross-service matching -- ISRC/merged-track aliasing, not raw track id). Removals upstream never come across; the library is the source of truth. This is how re-syncing an upstream playlist works now that import no longer touches playlists.
