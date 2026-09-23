---
id: canon-7f16
title: 'Cleanup from the arch review: duplicate decode paths, helpers, dead types, stale docs'
type: chore
priority: 3
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T20:41:36Z'
labels:
- arch
verify: cargo build --workspace && cargo test --workspace
---

From canon-ba30 cleanup list. Independent and low-risk:
- canon-audio has three decode paths: decode.rs decode() (the c4c3 spike), output.rs play_blocking (v1 driver), and engine.rs Decode. Collapse onto engine::Decode and keep the spike as a test helper; play-file can run through AudioPlayer.
- lan_ip duplicated (controller.rs, daemon cast.rs). codec_hint duplicated (controller.rs, main.rs, cast.rs), unless canon-ba9d absorbs it.
- LOCAL_SINK in player.rs duplicates SinkInfo::LOCAL.
- canon_core::Event is unused.
- Dev subcommands play and cast predate canon control, and cast duplicates controller wiring. Keep resolve/devices; drop play/cast once control covers them (update AGENTS.md harness list).
- Stale docs: canon-audio lib.rs says the local sink is a canon_core::Sink; canon-api lib.rs says the queue lands later; canon-sink lib.rs describes OutputRoute as live.

---
▸ 2026-09-23T20:41:33Z [Joel Webber]
Done:
- Removed `canon cast` (the one-shot Cast harness, fully covered by `canon control`) and `canon play` (a Tidal track on local, likewise). This also removed the duplicate lan_ip and two of the three codec_hint copies; the last one, in controller.rs, moves next to StreamInfo with canon-ba9d.
- `canon play-file` kept (the only way to play a local file today) but now runs the real engine (AudioPlayer, Output::Local) instead of the v1 blocking driver. canon-audio output.rs (play_blocking, PlayStats) deleted; PlayError moved to its own module.
- The decode.rs fMP4 check is kept on purpose: it is the entry point for tests/decode_fmp4.rs, which keeps the Tidal-container assumption checked.
- canon_core::Event (never used) deleted. CastSink::name/media_session and their fields, only used by the removed harness, deleted, as was the name parameter of CastSink::connect.
- Stale docs fixed: canon-audio lib.rs (the local sink is not a Sink; module map rewritten), canon-api lib.rs (the queue has landed), AGENTS.md harness list, the arch.md CLI line. (canon-sink lib.rs was already fixed in canon-583a; LOCAL_SINK went in canon-583a too.)
Evidence: green bar clean (fmt, clippy zero warnings, all tests, build). `canon play-file target/glass.flac` (a stereo FLAC made with afconvert from Glass.aiff): "output at 48000 Hz", "done.", exit 0.
Found while checking play-file, filed rather than fixed: mono tracks cannot play on the local output (canon-154e), and a local .m4a with its moov at the end cannot be opened (canon-6a92).

---
▸ 2026-09-23T20:41:36Z [Joel Webber]
verify: `cargo build --workspace && cargo test --workspace` -> PASS (exit 0)
