---
id: canon-e828
title: The librespot session dies when Spotify closes its connection, and Spotify-only tracks then fail instantly
type: bug
priority: 1
created: '2026-09-27T23:42:49Z'
updated: '2026-09-27T23:42:49Z'
labels:
- spotify
---

Seen 2026-09-27 23:38 on Joel's daemon: Persimmon Wake (149s) ended at 148.95s, the next entry 'Excerpt from the Hope' (bound only to spotify:3Xjs4XDLWlueVBddtq82O0) went Loading -> Error in 3ms. No flow join either: its preload failed the same way. SpotifyAudio (canon-librespot/src/lib.rs) makes one Session at startup and never replaces it, and librespot-core 0.8 session.rs:128 says an unexpectedly closed connection needs a new Session (Session::is_invalid() reports it). librespot logs 'ERROR librespot_core::session: Connection to server closed.' when it happens: a canon-9c73 test daemon logged it ~35 min after start (22:04). Fix direction: when is_invalid(), reconnect from the cached credentials (cache.credentials()) before Track::get, behind a lock so concurrent opens share one reconnect. Verify on the real account by waiting out a disconnect (or forcing session.shutdown()) and then playing a Spotify-only track.
