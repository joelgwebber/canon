---
id: canon-e828
title: The librespot session dies when Spotify closes its connection, and Spotify-only tracks then fail instantly
type: bug
priority: 1
created: '2026-09-27T23:42:49Z'
updated: '2026-09-27T23:46:43Z'
labels:
- spotify
---

Seen 2026-09-27 23:38 on Joel's daemon: Persimmon Wake (149s) ended at 148.95s, the next entry 'Excerpt from the Hope' (bound only to spotify:3Xjs4XDLWlueVBddtq82O0) went Loading -> Error in 3ms. No flow join either: its preload failed the same way. SpotifyAudio (canon-librespot/src/lib.rs) makes one Session at startup and never replaces it, and librespot-core 0.8 session.rs:128 says an unexpectedly closed connection needs a new Session (Session::is_invalid() reports it). librespot logs 'ERROR librespot_core::session: Connection to server closed.' when it happens: a canon-9c73 test daemon logged it ~35 min after start (22:04). Fix direction: when is_invalid(), reconnect from the cached credentials (cache.credentials()) before Track::get, behind a lock so concurrent opens share one reconnect. Verify on the real account by waiting out a disconnect (or forcing session.shutdown()) and then playing a Spotify-only track.

---
▸ 2026-09-27T23:46:43Z [Joel Webber]
Joel's log settles it (2026-09-27): 23:38:03 preload at 119.7s of 149: Track::get OK (spclient, HTTP), then 'ERROR librespot_core::session: Broken pipe (os error 32)', 23:38:05 'Audio key response timeout' -> 'spotify refused audio: spotify refused the audio key ... audio key response timeout'. Second bug: every audio-key failure maps to Error::Auth, Observed::open records it as a refusal, and grants() then withholds Capability::Stream, so at 23:38:33 Sources::open skipped librespot entirely and failed as unplayable in 3ms. The refusal clears only on a successful play or re-sign-in, and nothing will try to play: Spotify-only tracks stay dead until 'connect spotify.librespot' or a restart. Fix needs both: rebuild an invalid session (is_invalid(), or on a transport error retry once on a fresh session) and only treat a real refusal as Auth, not a timeout or a dead connection.
