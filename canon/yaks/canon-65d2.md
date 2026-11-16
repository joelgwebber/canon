---
id: canon-65d2
title: 'Server auth: player tokens and signed stream URLs'
status: shorn
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T20:56:02Z'
parent: canon-88a7
depends_on:
- canon-973d
source: arch
verify: cargo test -p canon-api auth && cargo test -p canon-api --test control_plane && cargo test -p canon-daemon audio::tests::a_paired_server
---

The API binds 127.0.0.1:7345 with no auth. Once players reach the server over the LAN (and later the internet) it needs auth, designed in now and used on the LAN too, so that internet exposure later means TLS in front, not a second code path.

- Pairing issues a long-lived player token; the token is also the player's identity (the per-player actor yak keys on it).
- The WS handshake carries the token; stream URLs carry short-lived signed tokens, since native players want a bare URL.
- A LAN bind option, and the canon-6227 firewall startup check applied to the server's ports.
- The player's own speaker-facing stream server stays unauthenticated on its LAN: speakers can't auth.

---
▸ 2026-10-17T20:40:51Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T20:48:38Z [delta:The Great Decoupling]

Progress (2026-10-17): player tokens done; stream-URL signing built, wired when canon-040e's endpoint merges.

Evidence:
- Green bar: fmt clean, clippy 0 warnings, test --workspace ok, build ok.
- Unit: HMAC-SHA256 matches `printf canon | openssl dgst -sha256 -mac HMAC -macopt hexkey:00..1f` (efbd97b1…); pairing names a player, re-pairing replaces, revoking works at once; players.json holds no token and is mode 0600; stream tokens: wrong key, edited claim, garbage and expiry all refused; the key persists across restarts.
- API tests: `a_paired_server_admits_paired_players_as_themselves`: 401 with no token or an unissued one; 403 for a token naming another player; admitted as `kitchen` with its token; 401 at once after revoke. `an_open_server_needs_no_token`: loopback unchanged.
- Live (own --state-dir with a library copy; LAN bind 192.168.0.27:7396): `canon pair kitchen` printed a 43-char token, players.json `-rw-------`; serve logged `not on loopback: only paired players may connect`; `canon control` without a token → `Http(Response { status: 401 …`; with `--token` → connected, status idle; `CANON_TOKEN=… canon tui --headless` → header `kitchen · local · connected`; ufw check for the control port now says `players can't connect … sudo ufw allow to any port 7396 proto tcp`.

Decision: access is decided by the bind address, not a flag. Loopback = Open (as today: whoever reaches it can read the state dir), anything else = Paired. Rejected: an `--auth` flag (a LAN bind with auth forgotten off is the failure to make impossible), and requiring tokens on loopback (breaks every local client and test harness for no gain).
Decision: tokens are 32 random bytes, base64url; only SHA-256 hashes are stored, re-read on each connection so pair/revoke apply without restart. Rejected: storing tokens (a readable file would grant access), JWT-style player tokens (revocation needs server state anyway).
Decision: pairing is a local CLI (`canon pair <player>`) on the server machine, writing the state dir. Rejected for now: an over-the-wire pairing flow (needs a trusted channel to bootstrap; a code-entry flow can come with the native player).
Decision: the token is the identity: a token's connection drives its player; `?player=` naming another is 403. Without a token on an Open server, `?player=` still works as before.
Decision: stream URLs use a stateless HMAC-SHA256 token (player, track, expiry; 15 min), hand-rolled HMAC over sha2 (already a workspace dep) checked against openssl. Rejected: the `hmac` crate (a new dependency for 15 lines), per-URL server state.
Decision: no TLS in canon. Documented: plain ws:// is for a trusted LAN; the internet goes through a TLS reverse proxy. Same code path, as the parent decided.

Remaining for shear: wire `StreamSigner` into canon-040e's endpoint seam (sign in the URL the server hands a player; verify on fetch, matching player + track), with a test. Waiting on canon-040e.

---
▸ 2026-10-17T20:56:02Z [delta:The Great Decoupling]

verify: `cargo test -p canon-api auth && cargo test -p canon-api --test control_plane && cargo test -p canon-daemon audio::tests::a_paired_server` -> PASS (exit 0)

---
▸ 2026-10-17T20:56:02Z [delta:The Great Decoupling]

Stream URLs wired (after merging canon-040e's endpoint):
- `ClientMessage::AudioUrl { track }` replies `ReplyData::AudioUrl { path: "/audio/<id>?token=…", expires }`, signed for the connection's own player and that track (15 min).
- `canon_api::audio::authorize` (the endpoint's one auth seam): a token must verify under this server's key and name this track (403 otherwise); no token is allowed only on an Access::Open (loopback) server, else 401.
- The key lives in `<state_dir>/stream.key` (0600), made on first start, kept across restarts.

Evidence: green bar (fmt clean, clippy 0 warnings, test --workspace ok, build ok); `a_paired_server_serves_audio_to_signed_urls_only` over a real listener: no token 401, another key's 403, another track's 403, right one 200 audio/flac; protocol round-trip covers `audio_url`. Live LAN checks of the WS side as in the note above.

Decision: an `audio_url` op rather than signing inside every Effect now: the executor role (canon-806d) will put signed URLs into the start effects it sends; until then a client asks. Rejected: a bearer header on /audio (native media stacks can't add one).

---
▸ 2026-10-17T20:56:02Z [delta:The Great Decoupling]

moved: shaving -> shorn
