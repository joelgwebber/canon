---
id: canon-bfe6
title: 'Protocol: a track detail op, service links in views, and control covering every op'
status: shaving
type: task
priority: 2
created: '2026-11-27T00:29:36Z'
updated: '2026-11-27T00:45:50Z'
parent: canon-1fb0
tags:
- api
- cli
---

[The third pass](/doks/design/tui-places.md), One protocol and Every kind has a detail: a Track detail op; each view carries its services' links (built by the daemon); a test that canon control reaches every client op.

#### 2026-11-27T00:29:37Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-27T00:30:34Z

*herd-player*

Claim (herd-player). Owner: worker `proto-detail`. Base: canon `5572553`. Design: [The third pass](/doks/design/tui-places.md), "Every kind has a detail" and "One protocol behind every interface".

## Scope

- **Track detail op:** `ClientMessage::Track { item: ItemRef }`, answered with `ReplyData::Track(TrackDetail)`. A new `TrackDetail` in `crates/canon-protocol/src/view.rs` holds the `TrackView`, every album it is on (`Vec<AlbumView>`), and its artists (`Vec<ArtistView>`). Put it in the library beside `Library::album`/`artist` (`crates/canon-library/src/lib.rs`), resolving an `ItemRef` the same way they do, so a service track not yet stored is ingested (stored, never saved). Dispatch it in `crates/canon-api/src/server.rs` beside `ClientMessage::Album`.
- **Service links:** every view that has `sources: Vec<SourceRef>` (`TrackView`, `AlbumView`, `ArtistView`, and `PlaylistView` if it has sources) gains `links: Vec<Link>`. A new `Link { service: Service, url: String }` in `view.rs`, `#[serde(default)]`. The daemon builds them, never the client: one function in `canon-library` (or `canon-core` if it fits there better: read where `SourceRef` lives, `crates/canon-core/src/id.rs`) maps a `SourceRef` and an entity kind to a public URL:
  - Tidal: `https://tidal.com/browse/{track|album|artist|playlist}/{id}`.
  - Spotify: `https://open.spotify.com/{track|album|artist|playlist}/{id}`.
  - Local: no link.

  Fill `links` wherever the views are built (`store.rs`'s `track_view`, `album_view`, `artist_view` …), so every reply carries them.
- **control reaches every op:** a test in `crates/canon-daemon` that lists every `ClientMessage` variant and asserts `canon control` has a command that sends it. A simple, robust shape: a `const` table in `control.rs` mapping each control command to the op name it sends, plus a test that every op name serde produces for `ClientMessage` (build one value of each variant, serialize, read `"op"`) appears in that table or in an explicit, argued `NOT_IN_CONTROL` list (for example `executor`/`become_executor`, which are a player's, not a person's). Add `track <item>` to `canon control`, printing the detail with each link. Add any other ops the test shows are missing, if each is small; otherwise list them under `NOT_IN_CONTROL` with a reason and say so in a note.
- Docs: `docs/arch.md` (the ops list, if there is one: grep `album`), and `canon control`'s `help` text.

Out of scope: TUI changes (the pane uses these in step 3); streamed replies and search (the other worker, `canon-9cc1`, edits `ServerMessage::Reply`, `PROTOCOL_VERSION` and the Search dispatch; don't touch those). You will both add to `ClientMessage`/`view.rs`: keep your edits to your own variants and fields.

## Evidence

- Library tests: the track detail of a stored track lists its albums and artists; a service `ItemRef` not stored is ingested and comes back with an id, and is not saved. Each view's links are right for Tidal and Spotify, and Local has none.
- The coverage test, passing, plus a deliberate check that it fails if a variant is added without a control command (say so in a note; don't commit the failing variant).
- Live: a test daemon on **7398, `--lan-port 7348`** (stop: `pgrep -f 'bind 127.0.0.1:739[8]' | xargs kill`; never touch 7345). Run `printf 'search army of me\ntrack 1\nquit\n' | ./target/debug/canon control --connect 127.0.0.1:7398` (or whatever selects a listed track) and paste the detail with its links.
- Green bar: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` (only the two old `firewall.rs` warnings), `cargo test --workspace`, `cargo build --workspace`.

Judge: the coordinator at landing.

#### 2026-11-27T00:31:31Z

*proto-detail*

Start: Fri Nov 27 00:31:26 UTC 2026
yaks path: /Users/joel/.local/share/yaks/farms/canon-b4dde6c5/canon/yaks/canon-bfe6.md
checkout: /Users/joel/Library/Application Support/delta/worktrees/x3axfak3dr0f/canon

#### 2026-11-27T00:45:48Z

*proto-detail*

Progress (00:5x UTC): disk filled mid-session (terminal dead); coordinator freed ~71 GiB. Resuming.

Done in tree so far (checked with `cargo check -p canon-protocol -p canon-library -p canon-api`, before the control changes):
- view.rs: `Link`, `TrackDetail`, `links` (serde default) on TrackView/AlbumView/ArtistView
- message.rs: `ClientMessage::Track`, `ReplyData::Track`
- canon-library: `link.rs` (`links(kind, sources)`), store.rs views fill it, `Library::track`
- canon-api server.rs dispatch; control.rs `track [item]` + help; docs/arch.md paragraph
Still to do: CONTROL_OPS table + coverage tests, library tests, failing-variant check, green bar, live run.

#### 2026-11-27T00:45:49Z

*proto-detail*

Decision: links are a computed field on each view, built from sources; rejected a stored column (no schema change, cannot go stale)

#### 2026-11-27T00:45:50Z

*proto-detail*

Decision: the link builder lives in canon-library (link.rs), not canon-core; it needs EntityKind from canon-protocol, and canon-core depends on nothing of ours
