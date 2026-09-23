---
id: canon-a73b
title: 'Library ops on the API: save/unsave, list and filter the saved library'
type: task
priority: 2
created: '2026-09-23T21:57:32Z'
updated: '2026-09-23T21:59:29Z'
parent: canon-4185
labels:
- library
- api
verify: cargo test -p canon-library saving_by_service_id
---

The saved set exists in the store but nothing can reach it. Ops: save/unsave {item} (any ItemRef; unknown service ids are ingested first), library {kind, query?, limit?, offset?} listing saved tracks/albums/artists newest first, filtered by title/name/credit. canon control: save [item] (no item = the current track), unsave, library [kind] [words].

---
▸ 2026-09-23T21:59:29Z [Joel Webber]
Built: Store::saved_page (kind, LIKE filter over title/name/credit with % and _ escaped, limit/offset, total); Library::entity_for (ingests unknown service ids: tracks by describe, albums/artists by fetching their listing), save, unsave, saved -> LibraryPage views. API ops save/unsave/library via a with_library helper. canon control: save [item] (bare = current track), unsave, library [kind] [words].

---
▸ 2026-09-23T21:59:29Z [Joel Webber]
Live 2026-09-23: save album:55391786 -> "1 saved albums: The Dark Side of the Moon — Pink Floyd"; play 33348478 then bare save -> "1 saved tracks: Army of Me — Björk · Post"; search bjork, save #21 -> "1 saved artists: Björk"; library tracks army -> 1, library tracks floyd -> 0; unsave 33348478 -> 0 saved tracks.

---
▸ 2026-09-23T21:59:29Z [Joel Webber]
verify: `cargo test -p canon-library saving_by_service_id` -> PASS (exit 0)
