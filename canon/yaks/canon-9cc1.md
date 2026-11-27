---
id: canon-9cc1
title: 'Protocol: streamed replies, and search everywhere on the server'
status: shaving
type: task
priority: 2
created: '2026-11-27T00:29:35Z'
updated: '2026-11-27T00:56:52Z'
parent: canon-1fb0
tags:
- api
---

[The third pass](/doks/design/tui-places.md), One protocol behind every interface: a Reply can say more is coming; Search with no service asks every connected catalog and streams one partial reply per service, then the merged final one.

#### 2026-11-27T00:29:36Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-27T00:30:33Z

*herd-player*

Claim (herd-player). Owner: worker `proto-stream`. Base: canon `5572553`. Design: [The third pass](/doks/design/tui-places.md), "One protocol behind every interface", decision 7.

## Scope

- `crates/canon-protocol/src/message.rs`: `ServerMessage::Reply` gains `more: bool` (`#[serde(default, skip_serializing_if = "is_false")]`, so old replies are unchanged on the wire). It means another reply to the same `id` follows; the last has `more: false`. Add `ServerMessage::partial(id, data)`. Bump `PROTOCOL_VERSION` to 2, and say in its doc comment what 2 added.
- `crates/canon-api/src/server.rs`: the connection's worker task (around line 240) sends one reply per request today. Let `dispatch` produce a stream of replies; a cheap shape is that `dispatch` gets a sender for partials and returns the final reply. `in_flight` counts requests, not replies, so decrement only on a final reply (`more: false`). Snapshot holding must keep working: snapshots wait while `in_flight > 0`.
- **Search everywhere:** `ClientMessage::Search { service: None }` asks every connected catalog, concurrently (one task each, through `Library::search` per service). Each service's answer goes out as a partial reply (`more: true`, `ReplyData::Search` holding that service's results) as soon as it arrives. The final reply is the merged `SearchView`: results deduplicated by entity id, which already merge across services because ingest matches ISRCs. A service that fails is skipped with a `tracing::warn!`, and its error goes in the final reply's view only if every service failed (then the reply is an error). `Search { service: Some(s) }` behaves exactly as today: one reply.
  - Today `browse_service` makes `None` mean "the default catalog". That changes: `None` means all. Check who sends `None` today: `canon control`'s `search` and the TUI. Both get "all" now, which is the intent; their tests may need updating.
  - You need a list of connected catalog services from `Sources` (`crates/canon-core/src/source.rs`). If no accessor exists, add a small one (`catalog_services()`) beside `default_catalog`.
- `crates/canon-tui/src/app.rs`: the TUI must keep working with partials. A reply with `more: true` must not remove the request from `pending`; fold partial search results into the Search page as they come, replacing them with the final view at the end. Keep this minimal: the Sources UI comes in a later step.
- `crates/canon-daemon/src/control.rs`: `canon control`'s `search` prints partials as they arrive (one line per service saying how many results came back), then the final list as today.
- Docs: `docs/arch.md`, where the control plane's messages are described (grep `Reply`): streamed replies and search everywhere.

Out of scope: the TUI's Sources view, Library filtering (step 5), and every file the other worker owns (`canon-bfe6`: the track detail op, service links in views, control's op-coverage test). If you must edit `message.rs`'s `ClientMessage` or `view.rs`, keep to your own additions; the other worker adds a `ClientMessage::Track` and a field on views, so expect a textual merge there and keep your edits local.

## Evidence

- Protocol tests: an old reply with no `more` deserializes as final, and `more: false` is not serialized.
- `canon-api` tests (`crates/canon-api/tests/control_plane.rs` has fakes): search with no service and two fake catalogs, one slow, gives a partial for the fast one first, then the other, then a final merged reply with the right `id`. A track found on both services appears once in the final reply. Snapshots are not interleaved before the final reply.
- Live: a test daemon on **7397, `--lan-port 7349`** (stop: `pgrep -f 'bind 127.0.0.1:739[7]' | xargs kill`; Joel's 7345 is never touched). Run `printf 'search opeth\nquit\n' | ./target/debug/canon control --connect 127.0.0.1:7397` and paste the per-service lines and the merged result. Tidal and Spotify are both connected in the shared state dir.
- Green bar: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` (only the two old `firewall.rs` warnings allowed), `cargo test --workspace`, `cargo build --workspace`.

Judge: the coordinator at landing, re-running the tests and the live search.

#### 2026-11-27T00:31:26Z

*proto-stream*

Start: 2026-11-27T00:31Z (date -u: Fri Nov 27 00:31:15 UTC 2026)
- yaks path: /Users/joel/.local/share/yaks/farms/canon-b4dde6c5/canon/yaks/canon-9cc1.md
- checkout: /Users/joel/Library/Application Support/delta/worktrees/wktnpdvjdx2h/canon

#### 2026-11-27T00:36:02Z

*proto-stream*

Decision: a partial search reply says which service it is from through a new `SearchView.service: Option<Service>` (None on the final merged view; skipped on the wire when None). Rejected: a new ReplyData variant (clients would need a second parse path for the same data), inferring the service from `sources` of the results (wrong for results already known on two services), and a field on Reply (Reply is generic; the service is search's business).

Decision: the merge rule lives once, in `SearchView::merge` (canon-protocol), used by the server for the final reply and by the TUI to accumulate partials. Dedup keys: entity id for tracks/albums/artists, (service, id) for playlists (which are never entities). Rejected: dedup in the server only (the TUI would then show duplicates while partials stream).

Decision: services are searched in a fixed order (sorted by name) for the merge, not completion order, so the final reply is deterministic; partials go out in completion order.

[live-search](canon-9cc1/live-search.log)

[live-dedup](canon-9cc1/live-dedup.log)

#### 2026-11-27T00:56:52Z

*proto-stream*

Evidence (all run in my checkout, 2026-11-27 ~00:56Z, after the disk was freed).

Green bar:
- `cargo fmt --all --check` -> exit 0
- `cargo clippy --workspace --all-targets` -> exit 0; only the two old firewall.rs warnings (enum `Port` never used, fn `check` never used)
- `cargo test --workspace` -> 31 `test result: ok` lines, none failed
- `cargo build --workspace` -> Finished

New tests (all pass):
- canon-protocol: `a_reply_is_final_unless_it_says_more` (old reply w/o `more` reads as final; `ack` does not write `more`; `partial` does), `merging_searches_leaves_out_what_is_already_there`
- canon-api tests/control_plane.rs: `search_everywhere_streams_a_partial_per_service_then_the_merge` (Tidal slow 400ms, Spotify fast 10ms, shared ISRC: frames are partial spotify, partial tidal, final; all id 7; no snapshot between even though another connection changed volume meanwhile; final holds the shared track once, in name order), `search_everywhere_skips_failures_and_a_named_service_is_one_reply` (one service down: still ok; named service: one reply, no `more`; all down: error reply). `hello.protocol` is now 2.
- canon-tui: `search_results_stream_in_and_the_final_reply_replaces_them` (partials fold in and the shared track shows once; the final reply replaces; a late reply after the final is ignored)

Live (daemon on 127.0.0.1:7397, --lan-port 7349, shared state dir, no schema change in this yak; stopped by `pgrep -f 'bind 127.0.0.1:739[7]' | xargs kill`, 0 left; 7345 untouched):
`printf 'search opeth\nquit\n' | ./target/debug/canon control --connect 127.0.0.1:7397` printed
  tidal: 40 results
  spotify: 37 results
then the merged list (15 tracks, 20 albums, 20 artists, 17 playlists). Full output attached: live-search.log.
Over --json (attached live-dedup.log): partial tidal 10/10/10/10, partial spotify 10/10/10/7 (tracks/albums/artists/playlists), FINAL no service 15/20/20/17. Tracks: sum of partials 20, unique ids 15, final 15, so the merge deduplicated 5 tracks found on both services.

Docs: docs/arch.md section 10 has a "Streamed replies" paragraph.

Summary: Reply gains `more` (absent on the wire when false); `ServerMessage::partial`; PROTOCOL_VERSION 2; `SearchView.service` + `len/is_empty/merge`; `Sources::catalog_services()`; server worker streams partials and `in_flight` drops only on a final reply; `Search{service:None}` searches all connected catalogs concurrently (JoinSet) and merges in name order, skipping failed services with a warn; TUI folds partials into the Search page and keeps the request pending; `canon control search` prints one line per service then the list.

Notes for the reviewer: the TUI and `canon control` both send `service: None`, so both now search everywhere (the intent). `browse_service` is still used by every other op, where None keeps meaning "the default catalog". The TUI test counts only the list column, because the pane beside it also shows the queue's "Era".
