---
id: canon-5fe5
title: There appears to be no affordance for deleting a local playlist
type: feature
priority: 1
created: '2026-10-01T12:23:37Z'
updated: '2026-10-01T13:33:17Z'
parent: canon-9c5d
depends_on:
- canon-28ff
labels:
- tui
- library
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

This makes it very difficult to clean up playlists merged from different sources.

Wire a key on a local playlist row (browse.rs's Item::Playlist, not Item::Remote/Album) to the existing playlist_delete op. Delete is the one genuinely irreversible action in this whole system -- it cascades away the playlist's version history too (canon-120d) -- so it needs a confirm step the TUI doesn't have any precedent for yet (no existing key currently asks 'are you sure'); design the smallest one that fits the existing single-key-per-row-action pattern (e.g. a second keypress to confirm, shown in the notice line). Depends on canon-28ff (refresh) landing first or alongside, so the playlist disappearing from the list doesn't need a tab re-entry to see.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.

---
▸ 2026-10-01T13:31:04Z [Joel Webber]
Built. D on a local playlist row arms the delete and says so in the notice line ("delete \"X\" and its history? D again to confirm, esc to cancel"); a second D sends ClientMessage::PlaylistDelete. esc cancels, in an arm placed before esc means "back", exactly as the merge cancel is. The armed state (App::deleting: Option<(EntityId, String)>) remembers WHICH playlist, so moving the cursor between the two presses arms the new row instead of deleting it; switching tabs disarms entirely, so an arm never lies in wait somewhere else. Scoped to Item::Playlist (or, from inside an opened one, Source::Playlist) -- Item::Remote and Item::Album get "only one of canon's own playlists can be deleted" and send nothing. The listing reloads afterwards (canon-28ff), so the row disappears on the spot.

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
▸ 2026-10-01T13:32:38Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)

---
▸ 2026-10-01T13:33:03Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
