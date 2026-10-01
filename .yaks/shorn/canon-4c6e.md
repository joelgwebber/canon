---
id: canon-4c6e
title: A reusable text-prompt widget in the TUI
type: task
priority: 2
created: '2026-10-01T13:08:15Z'
updated: '2026-10-01T13:33:17Z'
parent: canon-9c5d
labels:
- tui
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

self.input: Option<String> (app.rs) exists only for the search box -- search_key() hardcodes Enter as 'run a search'. Renaming a playlist and naming a duplicate (both below) each need the same keystroke-collection UI for a different purpose.

Generalize it: carry what the prompt is for (search / rename / name-a-copy / ...) alongside the string, pre-fillable (rename wants to start from the current name), and dispatch on Enter by that tag instead of assuming search. Keep it the one text-entry mechanism in the app rather than growing a second parallel one.

---
▸ 2026-10-01T13:15:51Z [Joel Webber]
Claimed as one serial lane (worktree wt/canon-9c5d-tui), build order: canon-4c6e (text prompt) -> canon-28ff (refresh) -> canon-5fe5 (delete+confirm) -> canon-2175 (rename) -> canon-ef86 (duplicate naming). canon-581d (optional) left unshaved; take it only if it falls out cheaply. Evidence: the verify command above.

---
▸ 2026-10-01T13:30:45Z [Joel Webber]
Built. app.rs: self.input: Option<String> becomes self.prompt: Option<Prompt>, where Prompt is { asking: Asking, text: String } and Asking is Search | Rename { playlist, was } | Copy { item: Option<ItemRef> }. search_key() becomes prompt_key(), which collects keystrokes exactly as before and on enter hands (asking, text) to answered() -- the one place that knows what each prompt means. ask(asking, text) opens it pre-filled, which is how rename starts from the name it is changing. An empty answer is still no answer: enter closes the prompt and asks for nothing. render.rs: search_line() becomes prompt_line() and the fixed "search: " chrome becomes Prompt::label() -- "search", "rename to", "copy as", "new playlist" -- so the box says what it wants. Still exactly one text-entry mechanism; nothing grew a second one.

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
▸ 2026-10-01T13:32:32Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)

---
▸ 2026-10-01T13:32:57Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
