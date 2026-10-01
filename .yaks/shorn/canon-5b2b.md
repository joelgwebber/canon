---
id: canon-5b2b
title: 'Track sources: service playlists/mixes as ItemRefs, Catalog::playlist(id), list a service''s playlists and mixes'
type: task
priority: 2
created: '2026-10-01T01:21:40Z'
updated: '2026-10-01T01:49:25Z'
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

---
▸ 2026-10-01T01:48:58Z [Joel Webber]
Built: Catalog::playlist(&SourceRef) -> SourcePlaylist in canon-core (default scans playlists(); Tidal overrides it with a direct /v1/playlists/<uuid> + /items fetch). canon-library: tracks_named expands ItemRef::Service{kind:Playlist} before locate can call it an artist, Library::service_playlists (ServicePlaylistView: service, id, name, track_count) and Library::service_playlist(ids, ingested). canon-api: ClientMessage::ServicePlaylists{service} + ReplyData::ServicePlaylists{playlists}, dispatched in server.rs beside Mixes. canon-daemon control.rs: `playlists [service]` lists them as queueable ItemRefs. Spotify deliberately keeps the default: a direct fetch of a merely-followed playlist comes back with no items, which would read as an empty playlist instead of a clear "not found".

---
▸ 2026-10-01T01:49:12Z [Joel Webber]
Live evidence against the real Tidal account, test daemon on 127.0.0.1:7399 (stopped by that port afterwards).

Shape of /v1/playlists/<uuid> checked first with `canon tidal-get` so the Tidal override is not a guess: it returns {uuid, title, numberOfTracks, ...} — the same object a page of /v1/users/<id>/playlists gives. Pinned as the test a_single_playlist_reply_reads_like_a_listed_one.

  printf "playlists\nquit\n" | canon control --connect 127.0.0.1:7399 --quiet
    #1   Fantasy (250 tracks)
    #2   Jazz-ish (130 tracks)
    #3   Jazz practice (5 tracks)
    ... 8 playlists

Copy (create_playlist) from a service playlist, end to end, then cleaned up:
  printf "playlists\npl new canon-5b2b scratch\npl add #3\npl show\npl delete\npl\nquit\n" | canon control ...
    playlist: canon-5b2b scratch (0 tracks)
    canon-5b2b scratch
      #1   Seven Sons of Bjorn - GoGo Penguin . Fanfares  5:18
      #2   Seven Hunters - Hidden Orchestra . Archipelago  9:37
      #3   Bardo (Live from Studio 2, Abbey Road Studios, London / 2020) - GoGo Penguin  6:00
      #4   The End of Dukkha (Special Edition) - Matthew Halsall  6:45
      #5   Take Five - The Dave Brubeck Quartet . Time Out  5:25
    (then `pl delete`; the final `pl` listing no longer holds it)

So the five tracks of the Tidal playlist "Jazz practice" came across in order into a canon playlist the user owns, through one ItemRef, with no code path special to playlists.

---
▸ 2026-10-01T01:49:20Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
