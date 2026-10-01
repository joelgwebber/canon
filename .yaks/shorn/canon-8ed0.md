---
id: canon-8ed0
title: 'Export: create a new upstream playlist from a local one'
type: task
priority: 3
created: '2026-10-01T01:21:55Z'
updated: '2026-10-01T02:17:24Z'
parent: canon-65f7
depends_on:
- canon-5b2b
labels:
- library
- api
needs: human
verify: cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace
---

export_playlist(id, service): creates a NEW playlist on the service from the local playlist's tracks (resolving each to that service's catalog), never updates or deletes an existing upstream playlist. Description stamped with something like 'Exported from canon, <date>' so repeat exports are easy to find and delete by hand. Returns a report of tracks that didn't resolve on that service. Re-export = nuke the old one upstream yourself and export again; canon does not delete upstream (an export log to support canon deleting its own copies later is deferred).

---
▸ 2026-10-01T01:57:33Z [Joel Webber]
Claimed for parallel lane canon-8ed0 (worktree wt/canon-8ed0). Scope: Library::export_playlist (lib.rs), a new write-capable seam on the service side, api/protocol.rs, control.rs. Hints: Capability::LibraryWrite already exists in canon-core/src/connection.rs and crates/canon-spotify/src/connector.rs:357 already asserts Spotify grants it (playlist-modify-private/public scopes are already requested) -- Tidal write support is unknown/unproven, treat it as optional/Unsupported for v1 if its API does not clearly support it. Mirror the existing Connector::catalog(&self) -> Option<Arc<dyn Catalog>> pattern (crates/canon-core/src/connection.rs ~L191-198) for a new write-capable accessor with a default None impl, so other connectors are not forced to implement it. Library::match_onto(sources, track, service) (lib.rs ~L895) already resolves a local track onto a target services SourceRef -- reuse it to resolve each track before creating the upstream playlist. Stamp the created playlists description noting it was exported from canon (canon-65f7 decision: canon never deletes upstream; re-export means the user deletes the old one by hand). Evidence: the verify command above. This is the most exploratory of the three parallel lanes -- investigate the real API shape (canon tidal-get, Spotify API docs) before committing to a design, and it is fine to land Spotify-only if Tidal write is not practical yet. Coordinator resolves any textual conflict in protocol.rs/control.rs against sibling lanes at merge.

---
▸ 2026-10-01T02:16:31Z [Joel Webber]
Built, Spotify-only. New seam: canon_core::Exporter (crates/canon-core/src/export.rs), create-only -- create_playlist(name, description, tracks) -> SourceRef, no update, no delete. Handed out like catalog(): Connector::exporter() defaults to None, Sources::exporter(service) / Sources::default_exporter() / Sources::with_exporter() route it. Added Connector::can_export() (default false) so routing can tell "sign in and it will work" from "canon does not speak this service write API" -- needed because Tidal advertises library_write it cannot back. Library::export_playlist(sources, id, service) -> ExportReport resolves every track with match_onto, skips what the service lacks (named in the report), refuses when nothing resolves (an empty upstream playlist is only litter the user must delete), and aborts on a lookup *error* before anything is created. Description stamped "Exported from canon, YYYY-MM-DD" (today_utc() in lib.rs, civil-from-days). API op export/Exported; control "pl export [service]".

---
▸ 2026-10-01T02:16:42Z [Joel Webber]
Tidal export DEFERRED, deliberately. Investigation: canon authenticates to Tidal as the leaked Android (6BDSRdpK9hqEBTgU) and TV clients with legacy scopes "r_usr w_usr w_sub". The official openapi.tidal.com/v2 POST /playlists needs playlists.write, which only a developer-portal client can be granted, and the two scope universes are mutually exclusive on the wire (a portal token is rejected by api.tidal.com/v1 with "Token is missing required scope. Required scopes: r_usr" -- tidal-music discussion #78). The reachable path is the unofficial v1: POST /v1/users/{id}/playlists (form) then POST /v1/playlists/{uuid}/items (form, trackIds=, onArtifactNotFound=SKIP) with an If-None-Match: <etag> header whose etag must be re-read from the response headers of GET /v1/playlists/{uuid} after every mutation. canon cannot do that today: TidalHttp has only get + post_form, HttpResponse drops response headers entirely, and canon has never POSTed to api.tidal.com at all (only auth.tidal.com). That is a new HTTP seam plus an ETag re-read protocol on an unsupported API, and it cannot be verified without writing real playlists to Joel live account that canon will never delete. Spotify needs none of it, so Tidal waits for its own yak.

---
▸ 2026-10-01T02:16:54Z [Joel Webber]
Evidence. Green bar, all four clean: cargo fmt --all --check (OK); cargo clippy --workspace --all-targets -- -D warnings (no output); cargo test --workspace -> 29 suites, every one "test result: ok", 0 failed; cargo build --workspace (Finished). New unit tests: canon-core source.rs why_an_export_is_refused_says_whether_signing_in_would_help (4 routing cases); canon-spotify export.rs x4 (private + stamped + URIs in order, >100 batched 100+1, a foreign binding refused before anything is created, a failed add names what was left upstream); canon-library lib.rs x4 (creates one playlist and names what it skipped, a second export creates a second playlist, nothing-to-export creates nothing, a service canon cannot write to is refused) + the stamp shape.

Live, test daemon on 7399 with an empty --state-dir (7345 untouched, never pkill -f "canon serve"):
  $ printf pl new export smoke\\npl export spotify\\npl export tidal\\npl export\\nquit\\n | canon control --connect 127.0.0.1:7399 --quiet
  playlist: export smoke (0 tracks)
  error: spotify cannot change your library: no Spotify client id: register an app at developer.spotify.com ...
  error: unsupported: canon cannot create a playlist on tidal
  error: unsupported: no service canon can create a playlist on is connected (`services` lists the ways)
So the op is wired protocol -> server -> control, and each of the three refusals is the honest one -- note Tidal says canon cannot, NOT "sign in", which is what can_export() buys.

---
▸ 2026-10-01T02:17:05Z [Joel Webber]
NOT VERIFIED against real Spotify, and blocked by something outside this yak. Joel spotify.web.json is present, so a live export was in reach, but no build can open the shared library at all right now: canon serve against the real state dir dies with "open the library ...: library: table playlist_versions already exists". The file is at PRAGMA user_version = 8 while migration 9 tables (playlist_versions, playlist_version_tracks) are already in it, so migration 9 re-runs and fails forever. Confirmed with sqlite3 directly; both 7345 and 7399 were down at the time, so nothing was holding it, and the failed open rolls back and changes nothing. That is canon-120d migration left half-applied in the shared state dir, not export, and I did not hand-repair Joel library. It blocks live verification for every lane until someone bumps user_version to 9 (or drops the two tables) -- worth its own yak. Consequence for canon-8ed0: POST /me/playlists and POST /playlists/{id}/items are implemented from the February 2026 migration guide (the old POST /users/{user_id}/playlists and /tracks paths now 403) and covered by scripted-HTTP unit tests, but no byte of this has been on the wire to Spotify.

---
▸ 2026-10-01T02:17:20Z [Joel Webber]
verify: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo build --workspace` -> PASS (exit 0)
