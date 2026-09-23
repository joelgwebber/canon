---
id: canon-16b7
title: Workspace, daemon skeleton & config
type: feature
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-23T04:33:21Z'
labels:
- architecture
verify: cargo build --workspace
---

The cargo workspace, the headless daemon process, its async runtime (tokio), structured logging, and lifecycle. Deliberately small crates with clear seams so no single file becomes the tideway monolith. This is the scaffold B-G hang off.

---
▸ 2026-09-23T04:33:13Z [Joel Webber]
verify: `cargo build --workspace` -> PASS (exit 0)

---
▸ 2026-09-23T04:33:21Z [Joel Webber]
Both remaining children shorn: 4103 (cargo workspace + crate layout) and 4a94 (daemon lifecycle + task supervision). canon-f04a was hoisted to top-level rather than shorn -- see its note; there are no settings yet for a settings schema to govern.

The scaffold claim was 'deliberately small crates with clear seams so no single file becomes the tideway monolith', and it has now been load-bearing for a while: six crates, all depending inward on canon-core, largest file ~850 lines. The seams held under the two changes most likely to have broken them -- adding a second position authority (canon-bc84) touched canon-core, canon-audio, canon-sink and canon-daemon without any of them reaching around the traits, and replacing the control client (canon-444b) touched one file and deleted a dependency.

Daemon lifecycle is exercised on every on-metal run: tokio runtime, structured tracing with per-target filters (RUST_LOG=canon_core::player=trace is how position reconciliation was verified), the axum control plane, discovery and cast sessions as supervised tasks, clean teardown on signal.
