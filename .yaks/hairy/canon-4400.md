---
id: canon-4400
title: Cancel in-flight resolve on rapid skip/seek
type: task
priority: 3
created: '2026-09-22T21:51:48Z'
updated: '2026-09-23T14:51:16Z'
parent: canon-b192
labels:
- audio,api
---

Today each play_index_at/seek holds the play through a full stream resolve (~1s) before the next can take effect; the generation guard keeps it correct (stale installs are discarded) but rapid next/prev/seek churn is serialized, so the winning track can be ~N*resolve late. Refinement: cancel the in-flight resolve/open_stream when a newer generation supersedes it (drop the resolve future / abort the producer spawn) so only the latest resolves. No correctness bug today (verified under hammering); this is a latency/feel improvement. Reduces wasted Tidal playbackinfo calls too.

---
▸ 2026-09-23T14:51:16Z [Joel Webber]
From canon-ba30: every seek (and every sink switch) goes through open_stream_at, which re-resolves Tidal playbackinfo + manifest before fetching a segment. A per-track manifest cache (canon-4c55 originally said manifest cache) would cut seek latency and shrink the window this yak is about. Consider doing both together.
