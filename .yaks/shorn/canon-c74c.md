---
id: canon-c74c
title: In-track seek via segment index
type: task
priority: 1
created: '2026-09-22T21:12:04Z'
updated: '2026-09-22T21:22:48Z'
parent: canon-b192
labels:
- audio
- tidal
verify: cargo test --workspace
---

Wire Command::Seek for streaming playback: map the target time to the DASH media-segment index (from the SegmentTimeline), reopen the stream from that segment, and set the clock to the segment start. Engine reports the start position in Loaded so the player resets+seeks the clock atomically (no reset-to-zero race). Segment-granular for v1; sample-accurate (decode+discard within the segment) is a later refinement.

---
▸ 2026-09-22T21:22:48Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-09-22T21:22:48Z [Joel Webber]
Shorn: in-track seek via segment index. DASH parse now captures per-segment durations (timescale-aware) -> segment_for_secs maps time to a media index; open_stream_at streams from that segment; Loaded carries start_ms so the player resets+seeks the clock atomically. Verified live over ws: seek to 120s jumps there and keeps advancing; seek back to 30s lands ~30s. Segment-granular (~4s); sample-accurate (decode+discard within the segment) is a noted refinement.
