---
id: canon-4054
title: Play a track with no streaming binding by matching it first
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:55:02Z'
parent: canon-880e
depends_on:
- canon-c739
- canon-b391
labels:
- library
- api
verify: cargo test -p canon-library -- matched_when_queued streamable_track_is_queued unmatched_track_is_queued without_a_streaming_service && cargo test -p canon-core what_can_stream
---

Step 3b. When a track's bindings include none on a streaming connection, match it onto the preferred streaming service before opening; surface playable-via (or no match) on track views.

---
▸ 2026-09-24T21:54:47Z [Joel Webber]
verify: `cargo test -p canon-library -- matched_when_queued streamable_track_is_queued unmatched_track_is_queued without_a_streaming_service && cargo test -p canon-core what_can_stream` -> PASS (exit 0)

---
▸ 2026-09-24T21:55:01Z [Joel Webber]
Built: matching at QUEUE time in the library. Library::playable(sources, tracks) passes a track with a streamable binding untouched (no network) and otherwise match_onto()s each browsable streaming service in Sources::streaming_services() order, returning the refreshed TrackRef; no ISRC / no match / lookup error (warned) leaves it as is so open() fails honestly (NotEntitled / no source). Duplicates in one call are looked up once. Wired into tracks_for, track_for, and autoplay top_up (crates/canon-daemon/src/autoplay.rs, one line). Playlist edits and save use tracks_named/bound_track (no matching: they play nothing).

---
▸ 2026-09-24T21:55:01Z [Joel Webber]
API: Sources::can_stream(service), Sources::streaming_services() (connectors granting Stream in registration order, then direct sources by name), Sources::plays_from(&[SourceRef]) -> Option<Service> (policy order shared with open via in_policy_order). TrackView gains plays_from: Option<Service>, marked in the library on every view-producing call (search, album, artist top tracks, track_views, playlist, create_playlist, saved). Signature changes: Library::track_views/playlist/saved now take &Sources; canon-api server.rs updated accordingly.

---
▸ 2026-09-24T21:55:01Z [Joel Webber]
Evidence: cargo test -p canon-library -- matched_when_queued streamable_track_is_queued unmatched_track_is_queued without_a_streaming_service -> 4 passed; cargo test -p canon-core what_can_stream -> 1 passed. Green bar: fmt --check ok, clippy --workspace --all-targets 0 warnings, test --workspace all ok, build ok. No live check: :7399 was held by another agent test daemon (left alone), and no Spotify connection exists to make a real cross-service match; unit tests are the evidence.

---
▸ 2026-09-24T21:55:01Z [Joel Webber]
Follow-up (not done): a persisted "no match on <service>" marker. Today an unmatchable track with an ISRC costs one catalog lookup per queue edit that includes it (deduped within the edit only). Needs a schema change; worth it once Spotify imports are real and large.
