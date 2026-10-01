---
id: canon-028b
title: TUI affordance for adding, removing, and moving items in a playlist
type: feature
priority: 1
created: '2026-10-01T17:30:13Z'
updated: '2026-10-01T17:33:22Z'
parent: canon-65f7
labels:
- tui
source: tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

At least as far as I can tell!

Investigated: the ops already exist (playlist_add/remove/move, since canon-6dc7) and canon control already has pl add/rm/mv. The Queue tab already has the exact pattern needed -- d removes the entry under the cursor, J/K move it -- in queue_key (app.rs), a handler entirely separate from the Playlists tab's page_key. Nothing wires equivalent keys to an opened local playlist.

Plan, reusing existing infrastructure rather than adding new:
- d and J/K inside an opened local playlist (Source::Playlist(id) specifically, not a service playlist/mix/album): mirror queue_key's remove and move_entry exactly, sending playlist_remove{playlist, index: cursor} and playlist_move{playlist, from, to}. Scope the same way D/R already do (local_playlist()).
- Reload after each is already covered: canon-28ff's reload_page()/Pending::Reload already treats Source::Playlist(id) as follows_playlists(), and its settle_cursor() already handles the row under the cursor vanishing (built for delete, applies identically to remove).
- Adding a track found elsewhere (search/library/an album) into a specific existing playlist: rather than a fourth key/flow, generalize the merge picker (M/start_merge/finish_merge, canon-e55d/canon-4b3b) to also accept a single Item::Track, not just a list -- list_source()/Item::list() currently only match Album/Playlist/Remote. playlist_add with merge:true on one track already means exactly 'add it if it's not there yet', the same mental model as merging a list, just singular. This is the one design call here; flag it if it turns out to read badly in practice and a separate key is warranted instead.
- docs/tui.md's key reference and Playlists section need the three keys (d/J/K read differently depending on tab, same as they already do between Queue and here).

---
▸ 2026-10-01T17:33:22Z [Joel Webber]
Claimed (worktree wt/canon-028b). Scope and plan as described above. Evidence: the verify command above.
