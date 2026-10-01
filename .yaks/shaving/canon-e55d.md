---
id: canon-e55d
title: 'TUI: browse remote playlists and mixes, copy and merge'
type: task
priority: 3
created: '2026-10-01T01:21:50Z'
updated: '2026-10-01T02:26:21Z'
parent: canon-65f7
depends_on:
- canon-5b2b
- canon-4b3b
labels:
- tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

Surface each service's playlists and mixes as browsable sources (mixes aren't in the TUI at all today). From a remote source: copy into a new local playlist, or merge into an existing one. From a local playlist: same two actions, so the UI doesn't need to special-case where a source came from.

---
▸ 2026-10-01T02:26:21Z [Joel Webber]
Claimed (worktree wt/canon-e55d). This is the last child of canon-65f7, solo now (no parallel sibling). IMPORTANT per canon-b3dc / AGENTS.md: if this touches schema.rs, live-check with its own --state-dir, never the shared real one. Scope: surface canon-5b2b service_playlists/mixes as browsable sources in the TUI, with copy (create_playlist) and merge (playlist_merge, canon-4b3b) actions; mixes are not in the TUI at all today (gap noted in the original canon-3db9 docs review). Evidence: the verify command above, plus regenerated docs/assets/tui-*.svg via cargo test -p canon-tui doc_frames -- --ignored if the layout changes.
