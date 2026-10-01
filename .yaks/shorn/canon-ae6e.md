---
id: canon-ae6e
title: Favorites import stops re-saving locally-unsaved items
type: task
priority: 3
created: '2026-10-01T01:22:08Z'
updated: '2026-10-01T01:46:31Z'
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

---
▸ 2026-10-01T01:46:18Z [Joel Webber]
Added schema migration 8: imported_favorites (entity, service, imported_at), the entity/service pairs an import has already offered. Store gains import_favorite(id, service, added_ms) -> bool (saves + records only the first time that pair is seen; returns whether it was) and was_imported(id, service). Library::import's three favorites loops call it instead of an unconditional save_at, so a re-import adds what is newly favorited upstream and never resurrects what the user unsaved locally. Not backfilled -- nothing records which service favorited what was saved before v8, and the first import after it only re-saves what is saved already. Tests: lib.rs a_re_import_leaves_what_the_user_unsaved_alone (import, unsave the track, re-import, still unsaved; the untouched artist stays saved) and store.rs an_imported_favorite_is_saved_once_and_then_left_to_the_user (per-service, unsave survives a re-offer, a second service's first offer does save). The two upgrade tests that rewind user_version now drop imported_favorites first, as the isrc one already did for identified/merged. docs/arch.md's import paragraph updated.

---
▸ 2026-10-01T01:46:26Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
