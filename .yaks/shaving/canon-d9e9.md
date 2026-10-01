---
id: canon-d9e9
title: Playlists in search results, and open a playlist by URL/id
type: task
priority: 3
created: '2026-10-01T01:22:03Z'
updated: '2026-10-01T01:57:33Z'
parent: canon-65f7
depends_on:
- canon-5b2b
labels:
- library
- catalog
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

SearchResults currently holds only tracks, albums and artists. Add playlists (and mixes where a service supports it) to search, and support opening a remote playlist directly by its URL/id (there's no open-by-id today). Spotify's algorithmic playlists have been unreachable via the API since late 2024, so this is about ordinary public/followed/owned playlists.

---
▸ 2026-10-01T01:57:33Z [Joel Webber]
Claimed for parallel lane canon-d9e9 (worktree wt/canon-d9e9). Scope: SearchResults gains a playlists field (canon-core/src/catalog.rs), Catalog::search impls in canon-tidal and canon-spotify, Library::search (lib.rs), SearchView/api/protocol.rs, control.rs search display, open-by-url/id. Hint: SearchResults already derives Default; canon-spotify/src/catalog.rs already builds it via SearchResults::default() (low risk), but canon-tidal/src/catalog.rs:366 and canon-library/src/lib.rs:1199 construct it as a full literal and need the new field added explicitly. Evidence: the verify command above. Coordinator resolves any textual conflict in protocol.rs/control.rs against sibling lanes at merge.
