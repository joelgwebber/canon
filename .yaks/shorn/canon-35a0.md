---
id: canon-35a0
title: Merge a recording the library holds twice
type: task
priority: 2
created: '2026-09-26T22:05:04Z'
updated: '2026-09-26T22:12:32Z'
parent: canon-880e
labels:
- library
verify: cargo test -p canon-library merge && cargo test -p canon-library twin && cargo test -p canon-musicbrainz merged_away
---

Two tracks with a shared ISRC (16 in Joel's library, from the ISRC case bug) or a shared recording MBID are one recording: fold the second into the first (bindings, ISRCs, album slots, playlist entries, saved, mbid), keep its id as an alias so clients and playlists holding it still resolve, and do it where a duplicate is discovered: at open for shared ISRCs, in the identifier for MBIDs, and when an ISRC match finds the copy bound to its twin.

---
▸ 2026-09-26T22:12:25Z [Joel Webber]
Live (test daemon :7399, real library, 2026-09-26): at open, 'merged 16 tracks held twice under one ISRC' (Queen's Berth, The Campsite, Friday, At the Gates, ...). With streaming.order briefly tidal-first (restored to spotify,tidal after), queueing Prog logged 'matched track 0546334e… onto tidal: tidal:23326022' — Siberian Khatru, via USEW20000054 learned from MusicBrainz in canon-882a. Before this, it and Queen's Berth were marked unmatched.

---
▸ 2026-09-26T22:12:32Z [Joel Webber]
verify: `cargo test -p canon-library merge && cargo test -p canon-library twin && cargo test -p canon-musicbrainz merged_away` -> PASS (exit 0)
