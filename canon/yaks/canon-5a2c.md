---
id: canon-5a2c
title: Daemon serves cached, resized album art
status: shaving
type: task
priority: 2
created: '2026-11-21T04:48:21Z'
updated: '2026-11-21T17:31:48Z'
parent: canon-3c5e
tags:
- tui
- api
needs: human
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
