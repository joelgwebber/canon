---
id: canon-e99d
title: Segment reader as MediaSource + expiry re-resolution
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T16:31:24Z'
parent: canon-94cc
labels:
- tidal
- audio
---

A streaming reader over the segment URL list that presents init+media as one non-seekable source to the decoder (symphonia MediaSource / io::Read). In-track seek = rebuild sliced from a segment index. THE FIX for tide-1100/bd9e: a 403/expired fetch re-resolves the manifest (via c2) and resumes at the current segment index instead of crashing. Interruptible fetch so track-change/seek teardown is fast.

---
▸ 2026-09-22T16:31:24Z [Joel Webber]
Buffered v1 landed in canon-tidal Source::resolve (fetch_all -> Cursor), proven by canon play. The STREAMING reader this yak is really about — segment-by-segment MediaSource with 403/expiry re-resolution and interruptible fetch (tide-1100/bd9e) — is still to do. Currently the whole encoded track is pulled before playback starts.
