---
id: canon-dfdd
title: PCM->FLAC encoder tap (PcmSink -> LAN stream broadcaster)
type: task
priority: 2
created: '2026-09-23T00:24:01Z'
updated: '2026-09-23T00:35:49Z'
parent: canon-dde4
labels:
- sink
verify: cargo test -p canon-sink
---

A canon_core::PcmSink impl that turns the engine's f32 PCM into a live FLAC stream feeding canon-sink's StreamBroadcaster. Precompute a static fLaC+STREAMINFO header (total_samples=0) for set_header; buffer to fixed blocks and encode each with flacenc::coding::encode_fixed_size_frame -> serialize the Frame (BitRepr/ByteSink) -> broadcaster.push. Offline-testable (feed a sine, assert header starts with fLaC and frames are emitted; ideally round-trip decode a block).

---
▸ 2026-09-23T00:35:48Z [Joel Webber]
verify: `cargo test -p canon-sink` -> PASS (exit 0)

---
▸ 2026-09-23T00:35:48Z [Joel Webber]
Shorn. Landed crates/canon-sink/src/flac_encode.rs (subagent, reviewed + integrated).
FlacTap implements canon_core::PcmSink: buffers interleaved f32 into 4096-frame blocks, f32->i32 scaled to bits_per_sample (clamp + round), encodes each block via flacenc::encode_fixed_size_frame, serializes the Frame (BitRepr->ByteSink), and pushes to a StreamBroadcaster. Constructor precomputes the 42-byte fLaC+STREAMINFO header (total_samples=0, min==max block size) and set_header()s the broadcaster once. flush() emits a valid shorter final frame from filled_size (no silence padding). Header metadata-block wrapper hand-framed byte-for-byte (flacenc MetadataBlock is pub(crate)); encode_fixed_size_frame is re-exported at flacenc crate root (not ::coding, which is pub(crate)).
Evidence: cargo test -p canon-sink (6 encoder tests incl. header layout=42B/last-block bit, block-boundary emission, partial-withhold, flush, multi-frame numbering, format-mismatch tolerance); clippy 0 warnings, fmt clean.
