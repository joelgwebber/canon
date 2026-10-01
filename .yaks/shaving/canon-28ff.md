---
id: canon-28ff
title: Refresh the current page after a playlist-mutating action
type: task
priority: 1
created: '2026-10-01T13:07:53Z'
updated: '2026-10-01T13:15:51Z'
parent: canon-9c5d
labels:
- tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

Copying or merging a playlist (canon-e55d) already leaves the visible listing stale -- the new or changed playlist doesn't show until the tab is left and re-entered, because Pending::Done just sets a notice (app.rs) and never re-runs the page's own request(). Rename and delete (canon-5fe5 and the rename yak) will have the same gap once built.

Fix it once, generically: after an op that changes what the current page is showing (copy, merge, and once they exist, create/rename/delete), re-issue that page's Source::request() in place -- the same request push_page already builds when opening a page fresh, just reused rather than re-pushing a new page onto the stack. No server change needed.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.
