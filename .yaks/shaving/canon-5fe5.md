---
id: canon-5fe5
title: There appears to be no affordance for deleting a local playlist
type: feature
priority: 1
created: '2026-10-01T12:23:37Z'
updated: '2026-10-01T13:15:51Z'
parent: canon-9c5d
depends_on:
- canon-28ff
labels:
- tui
- library
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

This makes it very difficult to clean up playlists merged from different sources.

Wire a key on a local playlist row (browse.rs's Item::Playlist, not Item::Remote/Album) to the existing playlist_delete op. Delete is the one genuinely irreversible action in this whole system -- it cascades away the playlist's version history too (canon-120d) -- so it needs a confirm step the TUI doesn't have any precedent for yet (no existing key currently asks 'are you sure'); design the smallest one that fits the existing single-key-per-row-action pattern (e.g. a second keypress to confirm, shown in the notice line). Depends on canon-28ff (refresh) landing first or alongside, so the playlist disappearing from the list doesn't need a tab re-entry to see.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.
