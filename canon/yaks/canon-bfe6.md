---
id: canon-bfe6
title: 'Protocol: a track detail op, service links in views, and control covering every op'
status: shorn
type: task
priority: 2
created: '2026-11-27T00:29:36Z'
updated: '2026-11-27T00:56:52Z'
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

#### 2026-11-27T00:45:51Z

*proto-detail*

Decision: PlaylistView gets no links: it has no sources field

#### 2026-11-27T00:45:52Z

*proto-detail*

Decision: play_track is under NOT_IN_CONTROL (queue_add supersedes it); enqueue is covered because --palette sends it from control.rs

#### 2026-11-27T00:45:53Z

*proto-detail*

Decision: the coverage test reads an explicit CONTROL_OPS table (command -> op) plus a code-vs-table check; rejected a bare grep of the source, which cannot say which command sends which op

#### 2026-11-27T00:46:49Z

*proto-detail*

Decision (revises the 4th): enqueue goes under NOT_IN_CONTROL too, not CONTROL_OPS: no command sends it (add sends queue_add); only the --palette flag does. Listing it as a command would make the table lie.

[fail1](canon-bfe6/fail1.log)

[fail2](canon-bfe6/fail2.log)

#### 2026-11-27T00:50:28Z

*proto-detail*

Deliberate failure check (probe variant `ClientMessage::ProbeOp` added, then reverted; not committed; `grep ProbeOp crates` is empty now):
1. variant only in message.rs: `cargo test -p canon-daemon control` fails to COMPILE, `error[E0004]: non-exhaustive patterns: ClientMessage::ProbeOp not covered` in canon-api's dispatch (target/fail1.log, attached).
2. variant added to server dispatch and to control.rs `every_op`/`listed` (the exhaustive match), but no command and no table row:
   `control_reaches_every_client_op` FAILED: "op `probe_op` has no `canon control` command: add one and list it in CONTROL_OPS, or argue for NOT_IN_CONTROL" (target/fail2.log, attached).
With the probe removed: 16 control tests pass.
Also fixed on the way: my first stale-row check read the wrong tuple field of NOT_IN_CONTROL (test failed on a reason string); now both tables are checked by op name.

[live](canon-bfe6/live.log)

[live2](canon-bfe6/live2.log)

#### 2026-11-27T00:56:43Z

*proto-detail*

Evidence (all run in the canon checkout, base 5572553 + this change; the coordinator can re-run each):

Library tests: `cargo test -p canon-library` -> `test result: ok. 74 passed; 0 failed`. New: `link::tests::each_kind_links_to_its_service_page_and_local_has_none` (Tidal + Spotify URL for all four kinds, Local none), `a_stored_tracks_detail_lists_its_albums_and_artists` (albums, artists, links on track/album/artist, same track by canon id and by tidal.com URL, an album id is rejected), `a_service_track_not_stored_is_ingested_and_not_saved` (returns an id, `bound()` finds it, saved total stays 0), `a_local_binding_has_no_link_beside_a_service_one`.

Coverage test: `cargo test -p canon-daemon control` -> `test result: ok. 16 passed; 0 failed` (incl. `control_reaches_every_client_op`, `the_control_table_matches_the_commands`). Failing-variant check is in the earlier note (fail1.log, fail2.log attached).

Green bar:
- `cargo fmt --all --check` -> exit 0
- `cargo clippy --workspace --all-targets` -> exit 0; only the two old firewall.rs warnings (`enum Port is never used`, `function check is never used`)
- `cargo test --workspace` -> exit 0, every `test result` line ok (74 canon-library, 67 canon-tui, 101 canon-api...), 0 failed
- `cargo build --workspace` -> exit 0

Live: test daemon `canon serve --bind 127.0.0.1:7398 --lan-port 7348` (shared state dir; no schema change), stopped afterwards by `pgrep -f 'bind 127.0.0.1:739[8]' | xargs kill` (0 left). Never touched 7345.
`printf 'search army of me\ntrack #1\n...' | canon control --connect 127.0.0.1:7398 --quiet` (full output: live.log, live2.log attached). Tail of `track #1` (the Bjork track as search listed it):
  Army of Me — Björk · Post  3:54
         tidal    https://tidal.com/browse/track/33348478
         spotify  https://open.spotify.com/track/3vEmpJQC2p8h0U3OiBufQm
  albums
    #1   Post — Björk (1995)
           tidal    https://tidal.com/browse/album/33348477
    #2   Post — Björk (1995)
           spotify  https://open.spotify.com/album/2Ul7B1LEHxXzYubtkTMENs
  artists
    #3   Björk
           tidal    https://tidal.com/browse/artist/16992
And a track not previously stored by id: `track 520285418` -> Backstabber, tidal links for the track (two tidal bindings: 520285418 and 520285450), its album and The Dresden Dolls. (The `error: unsupported: that is a album, not an artist` line in live.log is my own script: after `track #1` the listing renumbers, so `artist #1` named an album. The op's kind check working as designed.)

Notes for the reviewer:
- Other tests touched: canon-tui tests.rs fixtures gain `links: Vec::new()` (six struct literals); nothing else in canon-tui.
- Two Post albums in the live output are one Tidal and one Spotify album the library has not merged (an existing matching gap, not this yak).
- docs: docs/arch.md paragraph; `help` text and the module header in control.rs list `track [item]`.

#### 2026-11-27T00:56:51Z

*proto-detail*

Summary: track detail op (ClientMessage::Track -> ReplyData::Track(TrackDetail)), Library::track (ingests, never saves), daemon-built Link on Track/Album/Artist views (canon_library::links), `track [item]` in canon control, and CONTROL_OPS/NOT_IN_CONTROL coverage tests with a compile-time exhaustive match. Follow-up for the TUI step: the pane can read links and TrackDetail; PlaylistView has no sources so no links.

#### 2026-11-27T00:56:52Z

*proto-detail · kind: move · from: shaving · to: shorn · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAIFM7JIBxBCA0aLJyrhJPreUvVwLftHG*

moved: shaving -> shorn
