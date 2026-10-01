---
id: canon-ef86
title: Duplicating a local playlist should ask for a new name
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

copy_list() (canon-e55d) names the new playlist after whatever it copied. That's right for a remote source -- 'Jazz practice' from Tidal should land as 'Jazz practice' -- but wrong for a local Item::Playlist: duplicating 'Road trip' silently makes a second playlist also called 'Road trip', which is the 'copying is really just duplicate' complaint in canon-5fe5.

When the source of c is a local playlist specifically (not a remote one, not an album), prompt for a name (canon-4c6e) instead of defaulting to the same one; a remote source keeps today's behavior unchanged.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.
