---
id: canon-1c6e
title: 'TUI phase 4: one history'
status: shaving
type: task
priority: 2
created: '2026-11-21T04:48:21Z'
updated: '2026-11-21T17:30:54Z'
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
