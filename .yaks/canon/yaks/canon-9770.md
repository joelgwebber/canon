---
id: canon-9770
title: One Tidal 4005 on playbackinfo kills streaming for the whole session and skips the queue
status: shaving
type: bug
priority: 1
created: '2026-11-08T15:36:52Z'
updated: '2026-11-09T18:44:00Z'
tags:
- tidal
---

Found 2026-11-08 while chasing canon-d24e. A queue of Tidal tracks played cleanly for 77 minutes on the j15r server; then one `playbackinfo` request got `401 subStatus 4005 "Asset is not ready for playback"` and canon decided Tidal had refused streaming for the whole login.

Server log (j15r, the only copy is the deleted-but-open `serve.log` of pid 309725, read via `/proc/309725/fd/1`):

- 15:15:01.194 `tidal refused playback: playbackinfo rejected: HTTP 401 ... 4005` on the prefetch of the next track
- 15:15:30.048 track 6 ended; the player moved to track 7 and every remaining queue entry was then skipped within ~30 ms each, five or more in a row: `skipping "Hidden, Hostile": auth: the server refused the track (403): tidal can't stream: Tidal refused playback for the browser login (tidal.pkce) ... Sign in again with connect tidal.pkce`
- 15:15:30.205 `state=Error position_ms=0`, and it was still `Error` at 15:34 with nothing retrying

The seam: `pkce_seen` records the first refusal and `hint()`/`grants(Stream)` in `crates/canon-tidal/src/connector.rs` then report the login as unable to stream. 4005 is also what Tidal's fingerprint/UA/client-version gate answers (see the comments in `crates/canon-tidal/src/stream.rs`), and may just as well be transient or an expired token. One 401 should cost one track (retry once, after a token refresh if that is the cause), not the process's whole ability to stream and the rest of the queue.

Open questions:

- Is it transient (retry works), a token that expired ~77 min in (no refresh seen in the log), or a gate that started refusing this client?
- Does Tidal playback work now, from this same server, without a restart?
- A skipped-everything queue ending in `Error` with no client-visible reason is itself a bug (canon-60fa).

---
▸ 2026-11-08T15:38:26Z [canon-j15r]

Evidence kept at `~/canon-logs/serve-pid309725-0958-1534.log` on j15r (the original server's log; its worktree had been deleted, so the file was only reachable through `/proc/309725/fd/1`).

A fresh process does not hit it: `canon resolve 33348478` from a new process at 15:37 resolved lossless with 59 segments, and the restarted server logged `tidal streaming confirmed (Some(Lossless))`. So it is not a revoked login or a permanent gate; it is transient, or state that lives in the long-running process (a token that expired in-process, or `pkce_seen` latching the first refusal). Restarted the server (pid 520061, from this worktree's build) to clear it.

---
▸ 2026-11-09T18:44:00Z [canon-j15r]
kind: move
from: hairy
to: shaving
thread: ksQQ3YaazyXkR9WZ-OQ-DusyzJLOAE20MJIBxBCA0aLJyrhJPreUvVwLftHG
---

moved: hairy -> shaving

---
▸ 2026-11-09T18:44:00Z [canon-j15r]

Second occurrence, 2026-11-09 17:48:57Z on the restarted server (pid 520061, up ~26h): same signature. First `playbackinfo` 401/4005 on the prefetch of the next track, then at 17:49:28 every remaining queue entry skipped within ~100 ms, player `state=Error`. The log is saved at `~/canon-logs/serve-520061-plain.log` (ANSI stripped).

Ruled out so far:

- **Not de-auth'd.** The refresh token is fine: a fresh `canon resolve 33348478` at 18:42Z refreshed the token (file written 18:42:39Z) and resolved lossless. The access token was also still valid at the time of the failure (file said expires 18:04:13Z, written 14:04:13Z, so a 4h lifetime; failure at 17:48).
- **Not a bad login or a closed gate.** Same binary, same state dir, same Tidal account streamed fine seconds later from a new process.

What differs between the failing server and a fresh process: the long-running one holds a `session_id` cached from `GET /v1/sessions` (`crates/canon-tidal/src/session.rs`, `session_context`) and only re-fetches it when `account()` happens to run again (library browsing, or at startup). Playback sends it as `sessionId=` on every `playbackinfopostpaywall` call; the comment in `stream.rs` says it is "the other half of the 4005 gate". A session id that Tidal has expired or tied to an older access token would give exactly this error. Testing that next.

Hazard to remember: any second process that refreshes (my `canon resolve`, any `canon tidal-get`) rotates the refresh token on disk under the running server; do not run those against the shared state dir while a long test is going unless needed.
