---
id: canon-c6cd
title: Spotify playlists all show "0 tracks"
type: bug
priority: 2
created: '2026-10-01T12:06:17Z'
updated: '2026-10-01T12:13:14Z'
labels:
- library
verify: cargo test -p canon-spotify catalog::
---

If this is an API perf issue, we could make providing track counts at the top-level optional for services.
Note that it doesn't update after opening each playlist.

---
▸ 2026-10-01T12:13:10Z [Joel Webber]
Root cause: wrong field name, not a perf tradeoff. PlaylistInfo deserialized the size-reference object from JSON key "tracks", but live-captured /me/playlists responses from the real account (RUST_LOG=warn + a temporary raw-JSON dump, removed after diagnosis) show the key is "items" — e.g. "items":{"href":...,"total":135} — matching the modules own February-2026-migration note that playlist entries moved from "track" to "item". #[serde(default)] let the mismatch through silently as 0 rather than erroring, which is exactly why canon-3118s fix (reading the count straight from the listing) surfaced it: the count was always being read from a field that was never there. Renamed PlaylistInfo.tracks to .items to match; also corrected the search_finds_playlists_sized_but_not_listed test fixture, which had the same stale key and would have hit the same bug the moment anything read its track_count. No perf tradeoff needed -- the field was free in the listing all along, just misnamed. The "doesnt update after opening each playlist" note was a symptom of the same bug (the listing never had a correct count to begin with), not a separate defect: live-verified "playlists spotify" now shows real counts (Sci-Fi 135, Fantasy 303, Metallic 213, ... Prog 14) matching the raw API totals exactly, and opening Prog shows its real 14 tracks. Full green bar passes.

---
▸ 2026-10-01T12:13:14Z [Joel Webber]
verify: `cargo test -p canon-spotify catalog::` -> PASS (exit 0)
