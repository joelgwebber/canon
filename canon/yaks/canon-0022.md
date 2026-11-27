---
id: canon-0022
title: 'TUI pass 3, step 6: undo for playlists'
status: shaving
type: task
priority: 2
created: '2026-11-27T00:25:45Z'
updated: '2026-11-27T01:33:10Z'
parent: canon-3c5e
depends_on:
- canon-1fb0
tags:
- tui
---

[The third pass](/doks/design/tui-places.md), One protocol (Undo). Restore a playlist's previous version: one key, one op. Gathers canon-1d6c.

#### 2026-11-27T01:13:30Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-27T01:13:31Z

*herd-player*

Claim (herd-player). Owner: worker `undo-1`. Base: canon `f800a39`. Design: [The third pass](/doks/design/tui-places.md), One protocol (Undo). Joel's case (canon-1d6c): a stray `d` removes a track from a playlist.

## Design, settled

- **`u` undoes the last change to one of canon's own playlists made in this session.** It restores the version before it with the existing `PlaylistRestore { playlist, version }` op (`crates/canon-protocol/src/message.rs`), which records the restore as a new version. No new op, no schema change.
- The app keeps a small stack, cap 20, of `(playlist id, version to go back to, what was done)`. It pushes when a mutating playlist request succeeds: remove (`d`/`x`), move (`J`/`K`), merge (`M`), add (`L`), rename (`R`). To know "the version before", read the playlist's current version before the change: `PlaylistVersions { playlist }` gives the list, so take the highest before sending the edit. Or, if the edit's reply already carries a version, use that minus one. Read what the replies hold before choosing, and record the choice as a Decision note.
- `u` pops one entry, sends the restore, says `undid: removed "Era" from "Road trip"` in the notice, and reloads the playlist's pages, as other playlist edits do (`follows_playlists`). With nothing to undo: `nothing to undo`.
- **Lowercase `u` is an exception to "lowercase only looks"**, argued in the exceptions test: it reverses a change rather than making a new one, and it needs to be as cheap to reach as the `d` it rescues. Bind it in `Ctx::Global`, so it works from any view.
- **Out of scope:** undoing a playlist delete (its version history goes with it, and it already asks twice), queue edits (the queue is the session's), saves (`*` toggles itself), and redo. Say each in `docs/tui.md`.

## Scope by file

- `crates/canon-tui/src/app.rs`: the undo stack, recording on success (in the reply handling for the playlist edit `Pending`s), and the `u` command.
- `crates/canon-tui/src/keys.rs`: `Undo => "playlist.undo"`, group "Change", `mutates: true`, the binding, and the argued exception.
- `crates/canon-tui/src/tests.rs`, the key-reference snapshot (review the frame before accepting it), and `docs/tui.md` (the keys table and a short Undo paragraph).

**Not yours:** the coordinator is restructuring the tabs (`Tab`, `Shelf`, `browse.rs`, `render.rs`'s tab row, `switch_tab`, `switch_listing`) in the same files right now. Stay out of those functions. Keep your `app.rs` edits to new functions, the reply branch for playlist edits, and one `run_cmd` arm, so the merge stays small.

## Evidence

- Tests:
  - `d` in a playlist then `u` sends `PlaylistRestore` with the version before, and the notice says what was undone.
  - Two edits, then `u` `u`, restores them in reverse order.
  - `u` with nothing says so.
  - A failed edit pushes nothing.
- One rendered frame of the notice after an undo: a headless run against your own test daemon on **7397, `--lan-port 7349`** (stop: `pgrep -f 'bind 127.0.0.1:739[7]' | xargs kill`; never 7345). The live library is shared, so make a scratch playlist first (`C`, name it `undo-test`), add a track to it (`L`), remove it (`d`), and `u`. Save the frame with `svg target/undo.svg` and `yaks attach` it. Delete the scratch playlist at the end (`D` `D`).
- Green bar: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` (only the two old `firewall.rs` warnings), `cargo test --workspace`, `cargo build --workspace`.

Judge: the coordinator at landing.

#### 2026-11-27T01:14:06Z

*undo-1*

Start: Fri Nov 27 01:14:00 UTC 2026
yaks path: /Users/joel/.local/share/yaks/farms/canon-b4dde6c5/canon/yaks/canon-0022.md
checkout: /Users/joel/Library/Application Support/delta/worktrees/yj2bh63e6z22/canon
disk: 79Gi free on /System/Volumes/Data

#### 2026-11-27T01:28:06Z

*undo-1*

Decision: how to know "the version before". Read what the replies hold first: every playlist edit replies a bare `Ack` (canon-api/src/server.rs), no version. So the app asks `PlaylistVersions` either side of the edit, in one burst (before, edit, after); the server answers a connection's requests in order (server.rs worker loop), so the first read is the state before and the last the state after. Undo is remembered only if after > before, which also means a no-op edit (merge of tracks already held, move onto itself: `record_version` writes nothing) and a refused edit push nothing. Rejected: highest version from a cached list (stale if another client edited), reply version minus one (replies carry none), a new op / returning the version from the edit (protocol change, out of scope).
Decision: rename is undone with `PlaylistRename { name: was }`, not `PlaylistRestore`. A rename writes no version (`rename_playlist` only touches `playlists.name`), so there is no version to restore. The claim note lists rename among the undoable edits; this is the one way to honour that without a new op. Rejected: dropping rename from undo (leaves a known hole), a new op.
Decision: `u` is bound in `Ctx::Global` and dispatched in the global `(_, PlaylistUndo)` run_cmd arm, not the page-only match: a first version in the page match passed the playlist tests but could not run from the queue or Settings, which my "nothing to undo" test caught.
Decision: the notice on a move now reads `moved "<track>"` as before; the undo notice is `undid: ` + the original notice, so each edit's own notice is the single source for what gets said back.
Decision: deleting a playlist drops its entries from the undo stack (its history goes with it; a restore would fail with not found).
Decision: undo is not re-recorded: `u` pops and sends the restore without pushing; no redo, per the claim note.

![undo](canon-0022/undo.svg "After d then u in scratch playlist undo-test: the notice reads 'undid: removed \"Army Of Me (Live)\" from \"undo-test\"' and the track is back (1 tracks) in the pane")

#### 2026-11-27T01:33:10Z

*undo-1*

Evidence (all run in my checkout, base f800a39):
- `cargo test -p canon-tui`: `test result: ok. 74 passed; 0 failed; 1 ignored`. New tests: u_puts_back_the_track_d_removed_and_says_what_it_undid, u_twice_undoes_two_edits_in_reverse_order, u_with_nothing_to_undo_says_so_and_asks_for_nothing, an_edit_that_failed_or_changed_nothing_leaves_nothing_to_undo, a_rename_is_undone_by_naming_it_what_it_was, only_the_last_twenty_changes_can_be_undone; lowercase_only_looks_bar_the_listed_exceptions carries the argued `u` exception. The test fake `Mine` now keeps real versions (numbered from 1, no version for an unchanged list), so the restore tests check restored contents, not a scripted constant.
- Key-reference snapshot: only change is "9 more" -> "10 more" (the new `u` row in Everywhere); accepted after reading the diff.
- Green bar: `cargo fmt --all --check` exit 0; `cargo clippy --workspace --all-targets` exit 0, only the two old firewall.rs warnings (`enum Port is never used`, `function check is never used`); `cargo test --workspace` exit 0, every result line ok; `cargo build --workspace` Finished.
- Live, own daemon `canon serve --bind 127.0.0.1:7397 --lan-port 7349` (shared state dir; no schema change in this yak), `canon tui --connect 127.0.0.1:7397 --headless --size 100x20`:
  - `C`, `undo-test`: `made "undo-test"`, Playlists (13).
  - search "army of me", `L`, enter on undo-test: `"Army Of Me (Live)" is in "undo-test"`, listing 1 tracks.
  - open it, `d`: `removed "Army Of Me (Live)" from "undo-test"`, listing 0 tracks, pane `nothing here`.
  - `u`: `undid: removed "Army Of Me (Live)" from "undo-test"`, listing 1 tracks, pane shows `Army Of Me … Björk 4:20` again.
  - rendered frame attached: target/undo.svg (`yaks attach` ok).
  - cleanup: `D` `D`: `deleted "undo-test"`, Playlists (12); daemon stopped by `pgrep -f 'bind 127.0.0.1:739[7]' | xargs kill`. Port 7345 untouched.
Docs: docs/tui.md keys table row for `u` and an Undo paragraph (what is undoable, cap 20, not persisted, out of scope: playlist delete, queue edits, saves, redo).

#### 2026-11-27T01:33:10Z

*undo-1*

Summary: u (Ctx::Global, playlist.undo, group Change, mutates, argued exception) pops a 20-deep session stack of (playlist, version to go back to | old name, what was done) and sends PlaylistRestore (or PlaylistRename for a rename). Edits go through App::edit_playlist, which reads PlaylistVersions either side of the edit and records an undo only if a version was added. Files: crates/canon-tui/src/{app,keys,tests}.rs, the key-reference snapshot, docs/tui.md. Coordinator-owned functions (Tab, Shelf, browse.rs, switch_tab, switch_listing, render tab row) untouched; app.rs edits are new functions, the reply arms, the playlist-edit call sites and one run_cmd arm.
