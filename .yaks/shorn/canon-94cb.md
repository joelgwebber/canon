---
id: canon-94cb
title: Fuzzy title+artist+duration matching as the persisted fallback
type: task
priority: 2
created: '2026-09-26T21:53:28Z'
updated: '2026-09-26T22:15:51Z'
parent: canon-880e
labels:
- library
verify: cargo test -p canon-library fuzzy && cargo test -p canon-library a_track_no_isrc_finds
---

When no ISRC matches onto a preferred service, search it by title+artist and bind a candidate whose normalized title and artist agree and whose duration is within a few seconds (provenance fuzzy, confidence from closeness). Persisted like any binding, so a re-source never re-fuzzes.

---
▸ 2026-09-26T22:15:49Z [Joel Webber]
Live (test daemon :7399, real library, 2026-09-26; streaming.order briefly tidal-first, restored to spotify,tidal): queueing Prog logged 'matched track ca1579dc… onto tidal: tidal:396316185' (Era, fuzzy 0.937; the studio cut, not the 451s live 396320062) and 'onto tidal: tidal:396316179' (Will o the Wisp, fuzzy 0.943). All 14 Prog tracks now bind Tidal (12 isrc, 2 fuzzy). canon resolve 396316185: flac 16/44.1, 341.6s (Spotify's copy: 341.613s).

---
▸ 2026-09-26T22:15:51Z [Joel Webber]
verify: `cargo test -p canon-library fuzzy && cargo test -p canon-library a_track_no_isrc_finds` -> PASS (exit 0)
