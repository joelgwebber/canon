---
id: canon-5b2b
title: 'Track sources: service playlists/mixes as ItemRefs, Catalog::playlist(id), list a service''s playlists and mixes'
type: task
priority: 2
created: '2026-10-01T01:21:40Z'
updated: '2026-10-01T01:38:28Z'
parent: canon-65f7
depends_on:
- canon-f917
labels:
- library
- catalog
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

Make a remote playlist or mix a first-class, read-only ordered-tracks source, same shape as a local playlist from the client's perspective: ItemRef::Service{kind:Playlist} and ItemRef::Mix already exist in the queue path -- extend playlist ops (create_playlist/playlist_add) to accept any ItemRef as a source, not just local entities. Add Catalog::playlist(id) (today there's only bulk playlists()). Add a way to list a service's own playlists and mixes for browsing (mixes already have Catalog::mixes/mix; playlists need a lightweight list-without-tracks variant alongside the existing bulk playlists()).

---
▸ 2026-10-01T01:38:28Z [Joel Webber]
Claimed for parallel lane canon-5b2b (worktree wt/canon-5b2b). Scope: canon-core catalog.rs (new Catalog::playlist(source) -> SourcePlaylist, default impl may scan playlists() so existing impls keep compiling; Tidal/Spotify may override for efficiency), canon-library lib.rs (extend tracks_named ~L120-169 to expand ItemRef::Service{kind:Playlist} like the existing Mix arm; add Library::service_playlists near mixes() ~L740-755), canon-api protocol.rs + server.rs (new listing op, mirror Mixes/Mix), canon-daemon control.rs (listing command, mirror the existing "mixes" case ~L305). Evidence: the verify command above. Coordinator resolves any textual conflict in protocol.rs/control.rs against sibling lanes at merge.
