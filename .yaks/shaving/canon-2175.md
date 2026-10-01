---
id: canon-2175
title: Rename a local playlist from the TUI
type: task
priority: 2
created: '2026-10-01T13:08:23Z'
updated: '2026-10-01T13:15:51Z'
parent: canon-9c5d
depends_on:
- canon-4c6e
labels:
- tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

playlist_rename already exists as an op (and in canon control as 'pl rename'); nothing in the TUI calls it. Wire a key on a local playlist row to the prompt from canon-4c6e, pre-filled with the current name, and send playlist_rename on Enter.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.
