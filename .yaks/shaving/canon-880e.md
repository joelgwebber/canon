---
id: canon-880e
title: Source bindings + resolution policy (ISRC/MBID)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-24T21:55:06Z'
parent: canon-4185
depends_on:
- canon-ba9d
labels:
- library
---

Each canon Track holds an ordered set of Source bindings { Local(path), Tidal(id), Spotify(id)... } with a stored confidence + provenance. Playback picks a source by policy (local > streaming), so a track survives losing/gaining any one source. ISRC (+ MBID when present) is the FIRST-CLASS join key, not a fast-path; fuzzy title+artist+duration is the persisted fallback so a re-source never re-fuzzes from zero (tideway discarded everything but the resulting Tidal id). Spotify/Deezer sources are nice-to-have, not required.

---
▸ 2026-09-23T21:09:15Z [Joel Webber]
Partly landed with canon-f7da (2026-09-23): ingestion already joins on ISRC (same ISRC under another id = same recording, provenance isrc) and bindings carry provenance + confidence. Remaining here: MBID lookup (MusicBrainz by ISRC/barcode) to fill the modelled mbid columns, fuzzy title+artist+duration matching as a persisted fallback, and choosing among bindings beyond local-first.

---
▸ 2026-09-24T21:55:06Z [Joel Webber]
canon-4054 shorn (2026-09-24): its last open child. Not shearing 880e: MBID lookup and persisted fuzzy title+artist+duration matching remain, plus a persisted no-match marker (see 4054 notes).
