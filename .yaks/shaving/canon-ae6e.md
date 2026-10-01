---
id: canon-ae6e
title: Favorites import stops re-saving locally-unsaved items
type: task
priority: 3
created: '2026-10-01T01:22:08Z'
updated: '2026-10-01T01:38:28Z'
parent: canon-65f7
depends_on:
- canon-f917
labels:
- library
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

Known bug found during the playlist design review: a re-import of favorites re-saves tracks/albums/artists the user has since unsaved locally. Favorites import should only add what's newly favorited upstream since the last import, never resurrect something the user removed locally.

---
▸ 2026-10-01T01:38:28Z [Joel Webber]
Claimed for parallel lane canon-ae6e (worktree wt/canon-ae6e). Scope: schema.rs (new migration: a table recording which entities have ever been imported as a favorite from a service, independent of the live saved table), store.rs (write to it on favorite ingest, consult it to skip resaving), lib.rs Library::import ~L446-472 (favorites loop only -- do not touch the removed playlist code). Evidence: the verify command above. Coordinator resolves any textual conflict in schema.rs against sibling lane 120d at merge (both append a migration).
