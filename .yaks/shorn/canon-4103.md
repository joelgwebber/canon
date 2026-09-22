---
id: canon-4103
title: Cargo workspace + crate layout
type: task
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T02:57:57Z'
parent: canon-16b7
labels:
- architecture
- rust
verify: cargo build --workspace
---

Stand up the workspace and the crate boundaries from the root's crate map. Define the core traits up front (Source, Sink, the player state types) in canon-core so peripheral crates depend inward only. No business logic yet — just the seams and a compiling skeleton.

---
▸ 2026-09-22T02:57:57Z [Joel Webber]
verify: `cargo build --workspace` -> PASS (exit 0)

---
▸ 2026-09-22T02:57:57Z [claude]
Built the cargo workspace (Rust 1.95, edition 2024, resolver 2) with 7 crates under crates/: canon-core (substantive) + canon-daemon (bin 'canon') + documented stubs canon-{tidal,audio,sink,library,api}, each carrying its responsibilities and owning-yak refs. canon-core defines the seams everything hinges on: error/Result; id (EntityId, Service, SourceRef, Quality); track (TrackMeta, ReplayGain, TrackRef, StreamInfo, Codec); state (PlaybackState, FrameClock [the atomic frames+epoch clock that fixes the tide-2f85 desync class], PlayerSnapshot); command (Command, Event); source (Source trait + MediaInput + ResolvedStream); sink (Sink + PcmSink + SinkId/Kind/Health). Wire durations are u64 ms for the ws+json plane. Evidence: 'cargo build --workspace' clean; 'cargo clippy --workspace --all-targets' zero warnings; 'cargo run --bin canon' starts the tokio runtime and logs the seq-0 idle snapshot. Peripheral crates use 'use canon_core as _;' until they implement their trait. Next: canon-4a94 (daemon lifecycle/supervision) and canon-e284/canon-5afb (the state actor over these types).
