---
id: canon-882a
title: 'Identify tracks and albums with MusicBrainz: fill MBIDs, learn sibling ISRCs'
type: task
priority: 2
created: '2026-09-26T21:53:27Z'
updated: '2026-09-26T22:04:44Z'
parent: canon-880e
labels:
- library
- musicbrainz
verify: cargo test -p canon-musicbrainz && cargo test -p canon-library identif
---

Background identifier: ISRC -> MusicBrainz recording (mbid + every ISRC MB lists for it, so a Spotify-only track can match a Tidal copy released under another ISRC), barcode -> release + release-group. 1 req/s, descriptive UA with no personal contact. Learned ISRCs clear unmatched markers. Collisions (two library tracks, one recording) are logged, not merged.

---
▸ 2026-09-26T22:04:37Z [Joel Webber]
Live (test daemon :7399, real library, 2026-09-26): first batch identified 35 recordings; 'Siberian Khatru - 2003 Remaster' (Spotify-only, unmatched on Tidal) learned USEW20000054 + USRH12402797 from MusicBrainz (both on Tidal: 23326022, 406265155) and its tidal unmatched marker was cleared. Barcode search 503'd under load; added 503 retry with doubling waits. Second run: 'musicbrainz: looked up 50 tracks (42 identified, 28 new ISRCs) and 50 albums', with the 503s recovered by retry. Found 16 recordings held twice (Tidal import + Spotify import, missed because Spotify sent lower-case ISRCs); schema v4 folds case, merging them is a follow-up yak.

---
▸ 2026-09-26T22:04:44Z [Joel Webber]
verify: `cargo test -p canon-musicbrainz && cargo test -p canon-library identif` -> PASS (exit 0)
