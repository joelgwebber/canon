---
id: canon-1c6e
title: 'TUI phase 4: one history'
status: shorn
type: task
priority: 2
created: '2026-11-21T04:48:21Z'
updated: '2026-11-21T17:46:54Z'
parent: canon-3c5e
depends_on:
- canon-032a
tags:
- tui
---

Design section One history: o and i, back and forward, in every view and focus.

#### 2026-11-21T17:30:02Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T17:30:03Z

*herd-player*

Claim (herd-player). Owner: worker `hist-1`. Base: canon `7b7ff8c`.

Design: [One history](/doks/design/tui-player.md#one-history). One stack of locations; `o` back, `i` forward, in every view and focus. Not persisted.

## Scope (crates/canon-tui only)

- `app.rs`: a `Location` (the view, focus, the pane's subjects as `(title, Source, cursor)` each, the view's list cursor; Settings and the queue keep their own cursors as they are) and a `History { back, fwd }`, capped (50). A **jump** pushes the location being left and clears `fwd`: a view switch (`Tab`, `1`-`4`, `,`), opening a subject (`open_subject`, from the list or the pane), committing a search. **Not** jumps: cursor movement, `Esc` (`back()` peels layers as today and stays distinct from history), focus moves, the outputs picker.
- Restoring a location re-reads its pages (`load_page`, as yaks re-reads a document on return) and puts each cursor back by index, clamped once rows arrive. A location whose subject can no longer load says so in the notice; don't invent recovery beyond that.
- `keys.rs`: `nav.back` on `o`, `nav.forward` on `i`, in `Global` (help: "history back" / "history forward"). Lowercase is right: they look, they don't change anything.
- State header: add `hist=<back>/<fwd>` (counts) so headless scripts can see it.
- `docs/tui.md`: the pane section and the Everywhere keys table.

Out of scope: anything outside `crates/canon-tui` and `docs/tui.md`; the album-art work (another worker, canon-5a2c); persisting history; mouse.

## Evidence

- Tests in `crates/canon-tui/src/tests.rs`: open an album from Library, an artist from Search, switch views, then `o` steps back through each (view, focus, pane subject, cursor), `i` forward again; a new jump after `o` drops the forward side; the cap holds.
- The `?` popup snapshot will change (two new global keys): review it, keep two columns at 80x16 (shorten a help line if it collapses to one; see toque yak-9d07).
- Live: headless run against your own test daemon on **port 7398, `--lan-port 7348`** (another worker uses 7397/7349; Joel's daemon is 7345, never touch it), stopped with `pgrep -f 'bind 127.0.0.1:739[8]' | xargs kill`. Script: `2 ] Enter 4 ... o o i` and paste the frame headers showing `hist=` and the restored subject.
- Green bar: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` (only the two old `firewall.rs` dead-code warnings allowed), `cargo test --workspace`, `cargo build --workspace`.

Judge: the coordinator, at landing, by re-running the tests and the headless script.

#### 2026-11-21T17:30:54Z

*hist-1*

Start: Sat Nov 21 17:30:48 UTC 2026. yaks path: /Users/joel/.local/share/yaks/farms/canon-b4dde6c5/canon/yaks/canon-1c6e.md. Checkout: /Users/joel/Library/Application Support/delta/worktrees/jp3rg7y7rww9/canon

#### 2026-11-21T17:42:27Z

*hist-1*

Milestone: implementation + unit tests done (app.rs `Location`/`Subject`/`History`, keys.rs `nav.back`/`nav.forward`, 6 tests in tests.rs, key-reference snapshot accepted after review: still two columns at 80x16, `o history back` / `i history forward` under Everywhere). docs/tui.md and docs/assets/tui-keys.svg updated. fmt clean; clippy only the two old firewall.rs warnings. Next: live headless run on 7398.

[hist-live2](canon-1c6e/hist-live2.log "Live headless run against a test daemon on 7398: frame headers show hist=N/M; frames 14, 17, 20 show the artist in the pane, o back to the search results (album in the pane, cursor 103), o back to the Library album pane")

#### 2026-11-21T17:45:10Z

*hist-1*

Decision: a `Location` holds `Subject`s `(title, Source, cursor)` plus tab and focus, not live `Page`s or rows. Rejected: keeping whole pages (stale rows, memory); the claim note asks for a re-read on restore.

Decision: restore re-reads with `Page::reload()`, so each cursor settles on the nearest item row once the rows arrive (the existing reload path). Rejected: a new "pending cursor" mechanism.

Decision: `o`/`i` on a view with no browsing stack (Queue, Settings) just switch the tab; the queue's and Settings' own cursors are untouched. A restore also resets `library_kind` / `shelf` from the list's `Source`, so `[` and `]` carry on from there.

Decision: a search commit is a jump only when Search already showed results; filling the empty view for the first time is not (the `/` key already jumped to the Search tab, so that jump is what `o` undoes). Rejected: always pushing (two history entries for one visible change).

Decision: the cap is 50, dropping the oldest from `back`; `fwd` is not capped separately (it can only hold what `back` held).

Decision: only `go_to` (view keys), `open_subject` and the search commit jump. `switch_tab` stays jump-free because `[`/`]`, merge and restore call it; so `[`/`]` listing switches and merge's shelf switch are NOT jumps (not named in the claim note; say if you want them).

Decision: `o` with nothing to go back to (or `i` with nothing forward) sets a notice ("nothing to go back to" / "nothing forward of here") and changes nothing.

Decision: the existing test `keys_follow_the_users_overrides_and_family` rebound `item.open` to `o`; that now collides with `nav.back`, so it uses `e`. Same intent.

Evidence (all in canon checkout):

- `cargo test -p canon-tui`: 57 passed, 1 ignored (doc_frames). New tests: `o_and_i_walk_the_places_jumps_left_and_cursors_come_back` (album from Library, artist from Search, view switches; `o` x4 then `i` x4 checks view, focus, pane subject and cursors), `a_new_jump_after_going_back_drops_the_forward_side`, `movement_escape_focus_and_the_picker_are_not_jumps`, `the_history_keeps_the_last_fifty_places` (80 jumps -> hist=50/0, 60 `o` -> 0/50), `a_place_that_can_no_longer_load_says_so`, `o_with_nowhere_to_go_says_so_and_changes_nothing`.
- Key-reference snapshot reviewed: still two columns at 80x16, `o history back` and `i history forward` under Everywhere ("… 7 more" now).
- `cargo test --workspace`: all ok, 0 failed. `cargo fmt --all --check`: clean. `cargo clippy --workspace --all-targets`: only the two old firewall.rs warnings. `cargo build --workspace`: ok.
- Live: test daemon `--bind 127.0.0.1:7398 --lan-port 7348`, stopped by `pgrep -f 'bind 127.0.0.1:739[8]' | xargs kill`. Script `2 ] j Enter 4 / opeth Enter G Enter snapshot o snapshot o snapshot o snapshot i i snapshot`, attached as hist-live2.log. Headers: library album in pane `hist=2/0`; after search + artist opened `tab=search focus=pane pane=1 rows=20 cursor=0 hist=4/0`; `o` -> `tab=search focus=list pane=1 rows=100 cursor=103 hist=3/1` (frame shows the album Traced in Air in the pane); `o` -> `tab=library focus=pane pane=1 rows=9 cursor=0 hist=2/2` (frame: Albums list + Traced in Air in the pane); `o` -> `tab=library focus=list pane=0 rows=200/229 cursor=1 hist=1/3`; `i i` -> back to `tab=library focus=pane ... hist=2/2` then `tab=search focus=list pane=1 rows=100 cursor=103 hist=3/1`.
- Not checked live: a subject failing to load (unit-tested only).

Docs parity: grep for `history|nav\.` across docs/, README.md and keys.rs found only docs/tui.md (History paragraph, Everywhere table, headless header example) and keys.rs; docs/assets/tui-keys.svg regenerated by `cargo test -p canon-tui doc_frames -- --ignored` (only that svg changed).

Summary: new `Location`/`Subject`/`History` in app.rs (cap 50, not persisted); jumps (view keys via `go_to`, `open_subject`, a search over existing results) push the place left and clear `fwd`; `nav.back` `o` / `nav.forward` `i` in Global restore the view, focus, list and pane subjects by re-reading each page with its cursor; Esc untouched; state header gains `hist=<back>/<fwd>`; docs/tui.md and the keys svg updated.

#### 2026-11-21T17:45:10Z

*hist-1 · kind: move · from: shaving · to: shorn · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHXRoZIBxBCA0aLJyrhJPreUvVwLftHG*

moved: shaving -> shorn

#### 2026-11-21T17:46:54Z

*herd-player*

Landed (herd-player): replayed b2eeef6 onto main after the art commit as canon 533e177, pushed. Coordinator re-ran the green bar (fmt ok; clippy only the 2 old firewall.rs warnings; canon-tui 57 passed; workspace tests ok; build ok) and a headless run on 7398: 2 ] Enter 3 Enter 4 then o o o i gave hist=5/0 -> 4/1 (playlists, pane) -> 3/2 (playlists, list) -> 2/3 (library, album in pane) -> 3/2. Open: whether [ ] listing switches should count as jumps (worker left them out).
