---
id: canon-9770
title: One Tidal 4005 on playbackinfo kills streaming for the whole session and skips the queue
status: hairy
type: bug
priority: 1
created: '2026-11-08T15:36:52Z'
updated: '2026-11-08T15:36:52Z'
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
