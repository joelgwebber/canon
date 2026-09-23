---
id: canon-dfdd
title: PCM->FLAC encoder tap (PcmSink -> LAN stream broadcaster)
type: task
priority: 2
created: '2026-09-23T00:24:01Z'
updated: '2026-09-23T00:24:01Z'
parent: canon-dde4
labels:
- sink
---

A canon_core::PcmSink impl that turns the engine's f32 PCM into a live FLAC stream feeding canon-sink's StreamBroadcaster. Precompute a static fLaC+STREAMINFO header (total_samples=0) for set_header; buffer to fixed blocks and encode each with flacenc::coding::encode_fixed_size_frame -> serialize the Frame (BitRepr/ByteSink) -> broadcaster.push. Offline-testable (feed a sine, assert header starts with fLaC and frames are emitted; ideally round-trip decode a block).
