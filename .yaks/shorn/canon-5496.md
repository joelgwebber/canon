---
id: canon-5496
title: 'Streaming preference: play from services in the user''s order, match onto the preferred one first'
type: task
priority: 2
created: '2026-09-26T19:49:14Z'
updated: '2026-09-26T19:54:49Z'
parent: canon-c6ac
labels:
- library
- audio
verify: cargo test -p canon-core the_preferred_service_plays && cargo test -p canon-library matched_onto_the_preferred && cargo test -p canon-library a_missing_match_is_remembered
---

Agreed 2026-09-26: a settings list (streaming.order, default tidal then spotify) that (1) orders a track's bindings when opening (local files first, then services in that order), and (2) drives queue-time matching: try each streaming service in order, keeping an existing binding there or matching by ISRC, before settling for a less-preferred one; so plays go to the service Joel knows pays artists best. Remember a no-match per (track, service) so a track the preferred service lacks isn't looked up on every queue; recheck after 30 days, since catalogs change.

---
▸ 2026-09-26T19:54:43Z [Joel Webber]
Built: Settings.streaming.order (StreamingSettings, default [tidal, spotify], rank(): local always 0, unlisted after listed; omitted from the file when default). Sources::with_preference(settings) reads it at each use: bindings open in preference order, streaming_services() and plays_from() follow it. Library::playable now matches each track onto every browsable streaming service ranked above the best one it can already play from (and not already bound there), in order, first match wins; misses remembered in the new unmatched table (schema v3), skipped for RECHECK_UNMATCHED (30 days). Daemon wires the preference; canon control prefer <service>...

---
▸ 2026-09-26T19:54:43Z [Joel Webber]
Live 2026-09-26, test daemon on :7399 over the real library: prefer spotify tidal wrote {"streaming":{"order":["spotify","tidal"]}} into settings.json, prefer tidal spotify restored the default (key dropped). Queued the imported Spotify playlist Prog (14 tracks) at volume 0 then stopped: tracks with a Tidal binding 1 -> 11, unmatched rows 0 -> 3 (library migrated to schema v3). Joel daemon (:7345) untouched.

---
▸ 2026-09-26T19:54:49Z [Joel Webber]
verify: `cargo test -p canon-core the_preferred_service_plays && cargo test -p canon-library matched_onto_the_preferred && cargo test -p canon-library a_missing_match_is_remembered` -> PASS (exit 0)
