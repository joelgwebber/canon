---
id: canon-28ff
title: Refresh the current page after a playlist-mutating action
type: task
priority: 1
created: '2026-10-01T13:07:53Z'
updated: '2026-10-01T13:33:17Z'
parent: canon-9c5d
labels:
- tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

Copying or merging a playlist (canon-e55d) already leaves the visible listing stale -- the new or changed playlist doesn't show until the tab is left and re-entered, because Pending::Done just sets a notice (app.rs) and never re-runs the page's own request(). Rename and delete (canon-5fe5 and the rename yak) will have the same gap once built.

Fix it once, generically: after an op that changes what the current page is showing (copy, merge, and once they exist, create/rename/delete), re-issue that page's Source::request() in place -- the same request push_page already builds when opening a page fresh, just reused rather than re-pushing a new page onto the stack. No server change needed.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.

---
▸ 2026-10-01T13:30:56Z [Joel Webber]
Built. A new Pending::Reload(String) sets the notice and then reads the page on screen again: App::reload_page() takes the page the tab is already showing, calls Page::reload() -- which clears its rows (so a paged source asks from offset 0 instead of appending the next page), drops total, and returns the same ClientMessage request() already builds -- and sends it under Pending::Page(id) with the EXISTING id, so the reply folds into the page that is there rather than orphaning. A reloading flag makes the fold settle the cursor where the user left it (Page::settle_cursor: the nearest item at or above where it was) instead of jumping to the top the way a freshly opened page does. copy_list and finish_merge switched from Pending::Done to Pending::Reload; rename, delete and create use it too.

One deliberate scope limit: reload_page skips pages whose source is not Source::follows_playlists() (Playlists, Playlist(id), Library(Playlist)). Copying a 250-track Tidal playlist from the Tidal shelf would otherwise flash "loading..." and spend a round trip upstream to redraw a listing that a canon-side create cannot have changed.

---
▸ 2026-10-01T13:32:22Z [Joel Webber]
Live evidence, whole lane. Test daemon on 127.0.0.1:7399 against the real state dir (no schema.rs change in this lane, so that is the right thing to check against), stopped by that port afterwards. Driven with `canon tui --connect 127.0.0.1:7399 --headless --size 100x12`, toque keys.

Before: `pl` lists 21 playlists -- and the real library is exactly the mess these yaks describe: Post Rock x3, Sci-Fi x2, Fantasy x2, Neoclassical x2, Jazz-ish x2, Metallic x2.

  key 3, key N      prompt reads   new playlist: _
  type zz probe                    new playlist: zz probe_
  key Enter         Playlists (22), "zz probe  0 tracks" at the TOP of the listing, notice
                    made "zz probe"              <- canon-581d + canon-28ff, no tab re-entry
  key R             rename to: zz probe          <- canon-4c6e, pre-filled (canon-2175)
  type -renamed, Enter
                    Playlists (22), row now "zz probe-renamed", notice
                    renamed "zz probe" to "zz probe-renamed"
  key c             copy as: zz probe-renamed copy   <- canon-ef86: a LOCAL playlist prompts
  key Enter         Playlists (23), both "zz probe-renamed copy" and "zz probe-renamed",
                    notice copied into "zz probe-renamed copy"
  key D             delete "zz probe-renamed copy" and its history? D again to confirm, esc to cancel
  key Esc           delete cancelled; still Playlists (23), nothing gone   <- canon-5fe5
  key D, key D      Playlists (22), the copy is gone, deleted "zz probe-renamed copy"
  key D, key D      Playlists (21), deleted "zz probe-renamed"

On the Tidal shelf (key 3, key ]): key R gives "only one of canons own playlists can be renamed",
key D gives "only one of canons own playlists can be deleted". No request sent either time, and
nothing armed.

After: `pl` again, diffed against the before listing -> byte-identical. The 21 real playlists are
exactly as they were; everything created was created and deleted by the TUI under test.

Unit tests: 8 new ones in crates/canon-tui/src/tests.rs over a fake daemon that actually applies
the playlist ops, so a reload returns what changed rather than a scripted constant -- the prompt
opening pre-filled for each purpose, a copy/rename/delete landing in the listing with no re-open,
rename retitling an opened page, delete needing two presses and esc calling it off, a local
playlist prompting while a remote one still does not, and esc leaving any prompt alone.
36 pass in canon-tui, 0 fail.

docs/tui.md: the Playlists paragraph and the Library/Playlists/Search key table get R, D and N,
the duplicate-asks-for-a-name rule, the two-press delete, and the reload-in-place note. PAGE_KEYS
in render.rs gets the same three keys. Doc SVGs: `cargo test -p canon-tui doc_frames -- --ignored`
re-run and every docs/assets/tui-*.svg came back byte-identical (git reports no change), so none
needed committing -- tui-keys.svg shows the Queue tab keys, which is why the PAGE_KEYS additions
do not appear in it.

---
▸ 2026-10-01T13:32:35Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)

---
▸ 2026-10-01T13:33:00Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
