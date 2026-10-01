---
id: canon-d9e9
title: Playlists in search results, and open a playlist by URL/id
type: task
priority: 3
created: '2026-10-01T01:22:03Z'
updated: '2026-10-01T02:20:24Z'
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

---
▸ 2026-10-01T02:17:57Z [Joel Webber]
Built. canon-core: SearchResults gains playlists: Vec<SourcePlaylist>, and SourcePlaylist gains track_count — a search names playlists without listing them, so tracks is empty there and the service's own count is the only size. canon-tidal: /v1/search asks for PLAYLISTS too; PlaylistInfo reads numberOfTracks and describes itself. canon-spotify: /search asks type=...,playlist; the playlists section is Page<Option<PlaylistInfo>> because Spotify pads a short page with null. canon-library: SearchView.playlists: Vec<ServicePlaylistView> (not ingested — a service playlist is never a library entity), service_playlist_view() shared with service_playlists(); ItemRef::from_url() turns a pasted tidal.com/open.spotify.com link (or spotify: URI) into the ref every playlist path already takes — last <kind>/<id> pair wins, locale and browse segments ignored. canon-api: ClientMessage::ServicePlaylist{item} -> ReplyData::Tracks, mirroring Mix: open-by-id for the one thing with no canon id. canon-daemon: search output gains a playlists section, item() accepts a pasted URL, and a new 'open <item>' shows a service playlist's tracks, an album, an artist or a mix.

---
▸ 2026-10-01T02:18:09Z [Joel Webber]
Live evidence against the real Tidal account, read with canon tidal-get before modelling (the shape is not a guess):

  canon tidal-get /v1/search query='pink floyd' types=TRACKS,ALBUMS,ARTISTS,PLAYLISTS limit=2
    sections: albums, artists, playlists, topHit, tracks, videos
    tracks 2 / albums 2 / artists 2 / playlists 2
    playlist: 4ba30155-d807-446f-9e53-b67520ccfe83 | Pink Floyd Essentials | 25 tracks |
              http://www.tidal.com/playlist/4ba30155-d807-446f-9e53-b67520ccfe83

So the types= string the code now sends really does fill a fourth section, and Tidal's own link for a
playlist is the www.tidal.com/playlist/<uuid> form ItemRef::from_url parses. Both pinned as tests
(a_search_finds_playlists_sized_but_not_listed, a_pasted_playlist_link_names_the_playlist).

Wire path, test daemon on 127.0.0.1:7402 with an empty --state-dir (stopped by that port afterwards):
  printf 'open https://tidal.com/playlist/4ba30155-...\nopen nonsense\nopen https://tidal.com/album/55391786\nquit\n' | canon control --connect 127.0.0.1:7402 --quiet
    error: tidal can't browse: sign in to Tidal (tidal.pkce, or tidal.device for browsing only)
    usage: open <playlist or album url | #n | canon id>  (paste a tidal.com or open.spotify.com link)
    error: tidal can't browse: sign in to Tidal ...
A pasted playlist URL therefore reaches the new service_playlist op and gets as far as the catalog;
it fails only on sign-in, which an empty state dir is supposed to do.

---
▸ 2026-10-01T02:18:17Z [Joel Webber]
BLOCKER, not caused by this yak and not repaired here: the real library at ~/Library/Application Support/canon/library.sqlite is wedged, so no daemon can open it and a signed-in end-to-end run was impossible. PRAGMA user_version reads 8, while all three objects migration 9 creates (playlist_versions, playlist_version_tracks, playlist_version_tracks_track) already exist and are empty. Every 'canon serve' against the real state dir dies with 'table playlist_versions already exists'. schema.rs::migrate is itself correct (DDL and the version bump share one transaction), so something applied migration 9's DDL without its bump — most likely two lanes' daemons racing the same file. Joel's own daemon on 7345 is not running and will hit this on its next start. Minimal repair is PRAGMA user_version = 9 on that file; left to the coordinator rather than written to Joel's library from a lane.

---
▸ 2026-10-01T02:18:30Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)

---
▸ 2026-10-01T02:20:24Z [Joel Webber]
Merge check against main@e46f190 (canon-4b3b, merge, landed after this lane branched): git merge main auto-merges with no conflict in protocol.rs, server.rs, control.rs, lib.rs or arch.md, and the full green bar passes on the merged tree. Trial merge aborted, so this lane stays one commit.
