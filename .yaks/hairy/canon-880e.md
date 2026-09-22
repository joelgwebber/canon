---
id: canon-880e
title: Source bindings + resolution policy (ISRC/MBID)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T02:01:39Z'
parent: canon-4185
labels:
- library
---

Each canon Track holds an ordered set of Source bindings { Local(path), Tidal(id), Spotify(id)... } with a stored confidence + provenance. Playback picks a source by policy (local > streaming), so a track survives losing/gaining any one source. ISRC (+ MBID when present) is the FIRST-CLASS join key, not a fast-path; fuzzy title+artist+duration is the persisted fallback so a re-source never re-fuzzes from zero (tideway discarded everything but the resulting Tidal id). Spotify/Deezer sources are nice-to-have, not required.
