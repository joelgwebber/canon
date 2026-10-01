---
id: canon-e55d
title: 'TUI: browse remote playlists and mixes, copy and merge'
type: task
priority: 3
created: '2026-10-01T01:21:50Z'
updated: '2026-10-01T02:41:57Z'
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

---
▸ 2026-10-01T02:41:18Z [Joel Webber]
Built, all in canon-tui. browse.rs: Source gains ServicePlaylists/ServiceMixes (listings) and ServicePlaylist/Mix (one opened, folded from ReplyData::Tracks like radio/similar); a new Item::Remote wraps either as a row with no EntityId, so Item::id() is now Option<EntityId> and saved() is None for it, the way Item::Playlist already was. A Shelf enum names what the Playlists tab lists -- mine, Tidal playlists, Tidal mixes, Spotify playlists (no Spotify mixes: Catalog::mixes is Unsupported there, so the pairing is never offered). ReplyData::ServicePlaylists/Mixes fold into rows; SearchView.playlists (canon-d9e9) now shows as a "Playlists" section, which the TUI was dropping on the floor.

UI decisions: [ ] cycles the shelves on the Playlists tab root, exactly as it cycles kinds on the Library tab root -- same guard (root page only), same hint in the crumb line. `c` copies the list under the cursor into a new canon playlist of the same name (playlist_create). `M` holds it and switches to canon own playlists as the picker, where enter takes one (playlist_add merge:true) and esc gives up before esc can mean "back" -- no new modal, the existing listing IS the picker, mirroring `pl use` + `pl merge` in control.rs. Both act on the selected row when it is a list (playlist, mix, album) and otherwise on the page own source, so `c` inside an opened Tidal playlist copies the playlist, not the one track under the cursor.

---
▸ 2026-10-01T02:41:32Z [Joel Webber]
Live evidence against the real Tidal and Spotify accounts, test daemon on 127.0.0.1:7399 (stopped by that port afterwards). No schema.rs change in this yak, so the shared state dir was the right thing to check against.

  canon tui --connect 127.0.0.1:7399 --headless --size 100x16, toque keys

  key 3 key ]            Tidal playlists   [ ] mine . tidal . tidal mixes . spotify
                           Fantasy        250 tracks
                           Jazz-ish       130 tracks
                           Jazz practice    5 tracks
                           ... 8, all real
  key ] again            Tidal mixes
                           My Daily Discovery   Songs by new and familiar artists inspired b...
                           My Mix 1             Hidden Orchestra, Yppah, Thievery Corporatio...
                           ... 9 mixes, which the TUI could not reach at all before
  key Enter              Tidal mixes > My Daily Discovery, 10 tracks with artists/albums/times
  key ] (from tidal)     Spotify playlists, 11 of them

Copy and merge, end to end, then cleaned up:
  cursor on Tidal "Jazz practice" (5)
  key c    -> copied "Jazz practice" into a new playlist
  key M    -> merging "Jazz practice": enter on a playlist to take it, esc to cancel
  key Enter-> merged "Jazz practice" into "Jazz practice"      (the same source again)
  cursor on Tidal "Primus" (16), key M, key Enter
           -> merged "Primus" into "Jazz practice"

  pl listing before: 19 playlists.  after: 20, exactly one new id
    NEW 0a57d7df-d8eb-4f4a-be3d-2d78d963bb27  21 tracks  Jazz practice
  5 + 0 + 16 = 21: re-merging the same upstream playlist added nothing, Primus added all 16.
  `pl show` listed the 5 GoGo Penguin/Hidden Orchestra/Brubeck tracks followed by 16 Primus ones,
  in source order. Then `pl delete` on that id; the listing is back to 19.

---
▸ 2026-10-01T02:41:40Z [Joel Webber]
Docs: docs/tui.md gets its own Playlists section (shelves, read-only, c and M), the c / M / [ ] rows in the Library, Playlists, Search key table, and a note that c and M act on the list the cursor is on or the list being shown. Frames regenerated with `cargo test -p canon-tui doc_frames -- --ignored`: a new docs/assets/tui-playlists.svg (Tidal playlists) and tui-search.svg redrawn with a Playlists section; both looked at with qlmanage. README Status now says browsing/copying/merging a service playlist works, with export called out as Spotify-only (Spotify is the only Connector::exporter; Tidal has none).

---
▸ 2026-10-01T02:41:47Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
