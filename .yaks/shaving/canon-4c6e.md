---
id: canon-4c6e
title: A reusable text-prompt widget in the TUI
type: task
priority: 2
created: '2026-10-01T13:08:15Z'
updated: '2026-10-01T13:15:51Z'
parent: canon-9c5d
labels:
- tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

self.input: Option<String> (app.rs) exists only for the search box -- search_key() hardcodes Enter as 'run a search'. Renaming a playlist and naming a duplicate (both below) each need the same keystroke-collection UI for a different purpose.

Generalize it: carry what the prompt is for (search / rename / name-a-copy / ...) alongside the string, pre-fillable (rename wants to start from the current name), and dispatch on Enter by that tag instead of assuming search. Keep it the one text-entry mechanism in the app rather than growing a second parallel one.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.
