---
id: canon-0022
title: 'TUI pass 3, step 6: undo for playlists'
status: shaving
type: task
priority: 2
created: '2026-11-27T00:25:45Z'
updated: '2026-11-27T01:13:31Z'
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
