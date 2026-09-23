---
id: canon-7f16
title: 'Cleanup from the arch review: duplicate decode paths, helpers, dead types, stale docs'
type: chore
priority: 3
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T14:51:02Z'
labels:
- arch
---

From canon-ba30 cleanup list. Independent and low-risk:
- canon-audio has three decode paths: decode.rs decode() (the c4c3 spike), output.rs play_blocking (v1 driver), and engine.rs Decode. Collapse onto engine::Decode and keep the spike as a test helper; play-file can run through AudioPlayer.
- lan_ip duplicated (controller.rs, daemon cast.rs). codec_hint duplicated (controller.rs, main.rs, cast.rs), unless canon-ba9d absorbs it.
- LOCAL_SINK in player.rs duplicates SinkInfo::LOCAL.
- canon_core::Event is unused.
- Dev subcommands play and cast predate canon control, and cast duplicates controller wiring. Keep resolve/devices; drop play/cast once control covers them (update AGENTS.md harness list).
- Stale docs: canon-audio lib.rs says the local sink is a canon_core::Sink; canon-api lib.rs says the queue lands later; canon-sink lib.rs describes OutputRoute as live.
