---
id: canon-5a2c
title: Daemon serves cached, resized album art
status: shorn
type: task
priority: 2
created: '2026-11-21T04:48:21Z'
updated: '2026-11-21T17:39:27Z'
parent: canon-3c5e
tags:
- tui
- api
---

Design decision 1. The daemon fetches artwork_url, caches and resizes it, and serves it over the control plane HTTP side; clients cache by URL and never reach a CDN.

#### 2026-11-21T17:30:02Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T17:30:03Z

*herd-player*

Claim (herd-player). Owner: worker `art-1`. Base: canon `7b7ff8c`.

Design decision 1 of [the player design](/doks/design/tui-player.md#decisions): the daemon owns album art. Settled here so the worker implements, not decides:

## Design

- **Route** `GET /art/{entity}?size=<px>` on the control plane (`crates/canon-api/src/server.rs` `router`), beside `/audio/{id}`. `entity` is a track's or an album's canon id: clients already have both ids (snapshot `TrackRef.id`, queue entries, `TrackView`/`AlbumView`), so no protocol change and no URL in the request. Keyed by id, never by URL: a URL parameter would make the daemon an open fetch proxy.
- **Ownership**, mirroring `audio.rs`: `canon-api` gets `art.rs` with the route, the status codes, an `ArtSource` trait (object-safe, boxed future) and `with_art(...)` on the server builder; the daemon implements it. `canon-api` takes no HTTP-client or image dependency.
- **Lookup**: entity → `artwork_url` through the library (a track's own, else its album's; read how `TrackRef` gets `artwork_url` in `canon-core/src/track.rs` and the library's store). Unknown entity or no art: `404`.
- **Sizes**: snap `size` up to a bucket in `64, 160, 320, 640` (absent: 320); serve a square-bounded JPEG (`image/jpeg`), aspect kept. `Cache-Control: public, max-age=31536000, immutable` (an entity's art at a size doesn't change; a changed URL is a different cache key on the daemon side).
- **Cache** on disk: `<state_dir>/art/<sha256(url)>-<bucket>.jpg`, written atomically (temp + rename); a hit never touches the network. Fetch the original with the `reqwest` the daemon already has; resize with the `image` crate (new dependency: you may add it to the workspace `Cargo.toml` and `crates/canon-daemon/Cargo.toml`, `default-features = false`, jpeg + png decoding and jpeg encoding only, and the resulting `Cargo.lock` change. No other manifest edits.) A fetch failure is `502` and caches nothing.
- **Auth**: as the WebSocket is admitted (`admit` in `server.rs`): an `Access::Open` server serves anyone; a `Paired` one wants a paired player's `Authorization: Bearer` token. Reuse, don't fork, that check.

Out of scope: `crates/canon-tui` (the art phase, canon-ed4b, consumes this later); the library schema (`schema.rs`: no migration); any other route; the other worker's history work (canon-1c6e).

## Evidence

- `canon-api` tests with a fake `ArtSource`: 200 + headers for a known entity, 404 unknown, bucket snapping, Paired refuses without a token.
- Daemon-side tests with a fake fetcher: a second request for the same entity and bucket is a cache hit (no fetch); different buckets are different files; a failed fetch caches nothing.
- Live, against your own test daemon on **port 7397, `--lan-port 7349`** (another worker uses 7398/7348; Joel's is 7345, never touch it; stop with `pgrep -f 'bind 127.0.0.1:739[7]' | xargs kill`): `curl -s -o target/art.jpg -w '%{http_code} %{content_type}\n' http://127.0.0.1:7397/art/<a real track id>?size=160`, then `file target/art.jpg` (JPEG, 160 on its long side), then the same request again showing it came from the cache (log line or timing). Get a real track id from the queue via `canon control --connect 127.0.0.1:7397 --json` or the library.
- Green bar: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` (only the two old `firewall.rs` dead-code warnings allowed), `cargo test --workspace`, `cargo build --workspace`.
- Docs: a short section in `docs/arch.md` (or wherever the control plane's HTTP routes are described: grep for `/audio/`).

Judge: the coordinator, at landing, by re-running the tests and the curl.

#### 2026-11-21T17:31:10Z

*art-1*

Start: art-1. `date -u`: Sat Nov 21 17:31:06 UTC 2026.

- `yaks path canon-5a2c`: /Users/joel/.local/share/yaks/farms/canon-b4dde6c5/canon/yaks/canon-5a2c.md
- checkout: /Users/joel/Library/Application Support/delta/worktrees/fepvb6e2fn3h/canon

#### 2026-11-21T17:31:48Z

*art-1 · kind: ask · needs: human*

asked: needs human

Question (art-1): `canon-daemon` has neither `reqwest` nor `sha2` in its `crates/canon-daemon/Cargo.toml` (checked: `grep -rn "reqwest\|sha2" crates/*/Cargo.toml` shows them only in librespot, musicbrainz, spotify, tidal, api). The claim note says "the `reqwest` the daemon already has", but it has none of its own, and the one allowed manifest exception is `image` only.

The daemon-side fetch + cache needs:

1. `reqwest.workspace = true` in `crates/canon-daemon/Cargo.toml` (already a `[workspace.dependencies]` entry, `default-features = false`, rustls; already in `Cargo.lock`, so no new crate).
2. `sha2.workspace = true` in the same file (also already in the workspace table and the lockfile) for `<sha256(url)>-<bucket>.jpg`.

Alternatives I would otherwise take: (a) fetch through a `fetch_bytes(url)` helper exported by an existing crate that owns a reqwest client (`canon-musicbrainz`'s client is rate-limited and MusicBrainz-specific, `canon-spotify::ReqwestHttp` is API-with-auth: both wrong for a CDN image), or (b) re-export a `sha256_hex` from `canon-api`. Both are worse than two lines of manifest.

Please approve adding `reqwest` and `sha2` (workspace = true) to canon-daemon, or say which alternative. Meanwhile I am doing everything that doesn't need them: the `canon-api` side (route, `ArtSource`, `with_art`, shared auth check), its tests, `image` in the manifests, the docs.

#### 2026-11-21T17:34:40Z

*art-1*

Progress (art-1), paused on my ask above (reqwest + sha2 for canon-daemon).

Done, uncommitted in my checkout:

- `canon-api`: `src/art.rs` (route, `ArtSource`, `SIZES`, `bucket`), `with_art`, `/art/{entity}` route; `admit` is now `pub(crate)` and the handler reuses it (no forked check); `status_of` shared from `audio.rs`.
- `canon-api` tests: bucket snapping (unit), 200 + headers + snapping, 404, 501 without a source, Paired refuses without a token (`tests/control_plane.rs`). `cargo test -p canon-api art`: 4 passed.
- Manifests: `image` 0.25 (default-features = false, jpeg + png) in the workspace `Cargo.toml` and `crates/canon-daemon/Cargo.toml`, with the `Cargo.lock` change. `cargo build -p canon-daemon` builds.

Not started, waiting on the ask: daemon module (fetch with reqwest, sha256 cache key, resize), wiring in `main.rs`, daemon tests, live check, `docs/arch.md`.

Decision: `with_art` takes only the source (no signer): art uses `admit`'s bearer check, so no signed-URL token as audio has. Rejected: reusing `authorize`'s track token (a token names one track; art is asked for by album or track and a client wants many).

Decision: size past 640 snaps down to 640, not 404/400. Rejected: error on oversize (a client with a big pane just gets the largest bucket).

Decision: no `size` is 320 and a bucket is a public const (`ART_SIZES`, `ART_DEFAULT_SIZE`) so the TUI can pick without a magic number.

#### 2026-11-21T17:35:01Z

*herd-player · kind: answer · needs: agent*

Approved (herd-player): add reqwest.workspace = true and sha2.workspace = true to crates/canon-daemon/Cargo.toml. The claim note was wrong that the daemon already had reqwest. Both are already in the workspace table and Cargo.lock, so no new crates. No other manifest edits.

#### 2026-11-21T17:35:10Z

*art-1 · kind: pickup*

picked up

art-1: approval received, resuming daemon side (reqwest + sha2 for canon-daemon).

![art](canon-5a2c/art.jpg "GET /art/<track>?size=160 from the test daemon on 7397: 160x160 JPEG")

#### 2026-11-21T17:39:27Z

*art-1*

Evidence (art-1), test daemon `--bind 127.0.0.1:7397 --lan-port 7349`, stopped by `pgrep -f 'bind 127.0.0.1:739[7]' | xargs kill`. Track id taken from `canon control --json` `queue`.

Fresh fetch, then the same request again (Tidal cover, track `42e98535-bcb9-40dc-a6d7-ff7151e9a93e`, `size=160`):

```
200 image/jpeg 0.303342s 5257B     <- fetch + resize + write
200 image/jpeg 0.001213s 5257B     <- cache hit
target/art.jpg: JPEG image data, JFIF standard 1.02, ..., baseline, precision 8, 160x160, components 3
content-type: image/jpeg
cache-control: public, max-age=31536000, immutable
```

Daemon log (`RUST_LOG=info,canon=debug`), one `fetched` line and then hits:

```
INFO  canon::art: art: fetched https://resources.tidal.com/images/bddf1064/.../640x640.jpg, cached .../canon/art/f121d6db...-160.jpg
DEBUG canon::art: art: cache hit .../art/f121d6db...-160.jpg entity=42e98535-...
```

Other requests: unknown id `404`; `size=50` snapped to a 64x64 JPEG (own fetch, own file); a Spotify-cover track (`i.scdn.co`) at `size=640` gave `200 image/jpeg`, 640x640. `<state_dir>/art/` holds `<sha256(url)>-<bucket>.jpg` only.

Green bar:

```
cargo fmt --all --check      -> exit 0
cargo clippy --workspace --all-targets -> exit 0; only the two old firewall.rs dead-code warnings (`Port`, `check`)
cargo test --workspace       -> exit 0, every `test result` ok (new: 4 canon-api art tests, 5 daemon art tests)
cargo build --workspace      -> exit 0
```

Docs parity: `grep -rn "/art/\|ArtSource" docs README.md crates --include=*.md` finds only the new `docs/arch.md` paragraph (the only place `/audio/` is described, so the only place `/art/` belongs); no CLI flag, TUI key or skill text changed.

Decision: `Fetch` trait in the daemon, `HttpFetch` the reqwest impl. Rejected: a real local HTTP server in tests (slower, flakier) and testing only `resize`.

Decision: the cache write is best-effort: a failed write logs a warning and the picture is still served. Rejected: failing the request (a read-only cache dir shouldn't break art).

Decision: a picture smaller than the bucket is re-encoded, never enlarged (aspect kept, `thumbnail` for shrinking). Rejected: always scale to the bucket (blurry upscaling, and the same bytes cached for every bucket).

Decision: originals are capped at 16 MiB (content-length and body) so a wrong URL can't fill memory; JPEG quality 85; PNG alpha is flattened via `to_rgb8`.

Decision: entity lookup is `Store::kind_of` then `track_ref().meta.artwork_url` (a track's album's) or `album().artwork_url`; any other kind (artist, playlist) is 404. Rejected: a new schema/column (out of scope).

Decision: failed fetch maps to `Error::Source` which `status_of` already sends to 502; reused, no new mapping.

Summary: `GET /art/{entity}?size=` in canon-api (route, `ArtSource`, `with_art`, shared `admit`), `ServerArt` in canon-daemon (library lookup, `<state_dir>/art/<sha256(url)>-<bucket>.jpg`, reqwest fetch, `image` resize, atomic write), wired in `run_serve`, `docs/arch.md` paragraph. Manifests: `image` (workspace + canon-daemon), `reqwest` and `sha2` (canon-daemon, approved), `Cargo.lock`.

Finish: `date -u` Sat Nov 21 17:39:14 UTC 2026 (started 17:31:06).

#### 2026-11-21T17:39:27Z

*art-1 · kind: move · from: shaving · to: shorn · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHXTVpIBxBCA0aLJyrhJPreUvVwLftHG*

moved: shaving -> shorn
