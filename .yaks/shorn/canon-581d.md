---
id: canon-581d
title: Create a new empty playlist from the TUI
type: task
priority: 3
created: '2026-10-01T13:08:30Z'
updated: '2026-10-01T13:33:17Z'
parent: canon-9c5d
depends_on:
- canon-4c6e
labels:
- tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

Not asked for -- a related gap noticed while scoping canon-9c5d. Today only canon control's 'pl new <name>' can make an empty playlist; the TUI's only playlist-creation path is copying a source into a new one (c). Once canon-4c6e's prompt exists, a key on the Playlists tab's own 'mine' listing (not on a row) could call playlist_create with an empty item list and the typed name. Low priority; do it only if it falls out cheaply alongside the others, or defer.

---
▸ 2026-10-01T13:31:29Z [Joel Webber]
Taken after all: it fell out of canon-4c6e in about ten lines. Asking::Copy already carried an Option<ItemRef>, so an empty playlist is the None case -- same prompt, same PlaylistCreate, same reload, just no items. N on the Playlists tab opens it (n is next-track, globally). Shaved late and honestly: the code was written while finishing the lane, before the shave.

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
▸ 2026-10-01T13:32:47Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)

---
▸ 2026-10-01T13:33:13Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
