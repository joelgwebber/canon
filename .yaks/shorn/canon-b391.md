---
id: canon-b391
title: Match a track onto another service by ISRC and bind it
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:35:09Z'
parent: canon-880e
labels:
- library
- tidal
verify: cargo test -p canon-library -p canon-tidal
---

Step 3a of docs/connections.md. Catalog gains a lookup by ISRC (Tidal: check what v1 offers with canon tidal-get, else search plus an ISRC check); Library::match_onto(entity, service) finds the service's recording with the entity's ISRC, ingests and binds it (provenance isrc), and reports no-match honestly. Independent of the connections work.

---
▸ 2026-09-24T21:34:50Z [Joel Webber]
Probe (canon tidal-get, real token): Tidal v1 answers ISRC lookup directly. `canon tidal-get /v1/tracks isrc=GBN9Y1100081 limit=100` -> {limit:100, offset:0, totalNumberOfItems:3}, items (id isrc duration allowStreaming streamReady album): 55391582 GBN9Y1100081 394 True True "A Foot in the Door: The Best of Pink Floyd"; 528916606 GBN9Y1100081 394 True True "8-Tracks"; 55391792 GBN9Y1100081 380 True True "The Dark Side of the Moon". Items are the full /v1/tracks/<id> shape. Lowercase isrc=gbn9y1100081 returns the same 3. isrc=QQ0000000000 -> {"items": [], "limit": 10, "offset": 0, "totalNumberOfItems": 0}. (isrc=ZZZZZ9999999 is a real track, Lopes FR "Longa Caminhada": do not use it as a no-match fixture.) /v1/search query=<ISRC> does NOT search ISRCs (returned unrelated tracks), so no search fallback. Note: the real ISRC of Money (55391792) is GBN9Y1100081, not ...086 as the fixtures used.

---
▸ 2026-09-24T21:34:50Z [Joel Webber]
Built: Catalog::tracks_by_isrc(&self, isrc: &str) -> Result<Vec<SourceTrack>> with a default Err(Unsupported) (canon-core/src/catalog.rs). Tidal impl: GET /v1/tracks?isrc=&limit=100, keeps only items whose isrc equals (case-insensitive) and that are not allowStreaming/streamReady=false. Store::bind_isrc_match(track, described) -> bool binds the NAMED track (not the first with that ISRC), refusing a copy whose ISRC the track lacks or whose binding another track owns; shares ingest_unbound with ingest_track. Library::match_onto(sources, track, service) -> Result<Option<SourceRef>>: existing binding on service returned as is; else each ISRC looked up, copies sorted by closeness to the track duration, first bindable one bound (provenance isrc) and ingested with its album; None for no ISRC / no copy; NotFound for a non-track. Not wired into playback or the API (canon-4054).

---
▸ 2026-09-24T21:35:01Z [Joel Webber]
verify: `cargo test -p canon-library -p canon-tidal isrc` -> PASS (exit 0)

---
▸ 2026-09-24T21:35:09Z [Joel Webber]
verify: `cargo test -p canon-library -p canon-tidal` -> PASS (exit 0)

---
▸ 2026-09-24T21:35:09Z [Joel Webber]
Evidence: cargo test -p canon-library -p canon-tidal -> canon-library 27 passed (incl. a_track_is_matched_onto_another_service_by_isrc, a_track_already_on_the_service_keeps_its_binding, no_isrc_or_no_such_recording_is_no_match, store::an_isrc_match_binds_only_the_named_recording), canon-tidal 19 passed (incl. an_isrc_lookup_keeps_streamable_copies_of_that_recording). Full green bar: fmt clean, clippy 0 warnings, cargo test --workspace all ok, cargo build ok.
