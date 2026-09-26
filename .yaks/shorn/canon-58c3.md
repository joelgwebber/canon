---
id: canon-58c3
title: 'spotify.librespot connection: Spotify audio in the daemon'
type: task
priority: 2
created: '2026-09-26T21:14:16Z'
updated: '2026-09-26T21:18:45Z'
parent: canon-c6ac
labels:
- spotify
- audio
verify: cargo test -p canon-librespot && cargo test -p canon-core source
---

The connector for the librespot spike: method spotify.librespot (Stream at High, Vorbis 320), paste-back PKCE sign-in (client 65b70807..., redirect http://127.0.0.1:5588/login, scope streaming) built by canon rather than librespot-oauth blocking on a listener, credentials cached in <state_dir>/spotify.librespot/, restore at startup, audio-key refusal -> Degraded and routed around. Sources gains several connectors per service (spotify.web for library, spotify.librespot for audio).

---
▸ 2026-09-26T21:18:42Z [Joel Webber]
Built: LibrespotConnector (spotify.librespot: Stream at High only; restore from librespot cached credentials at startup, never fatal; paste-back PKCE login built by canon: authorize URL with S256 challenge + state, scope streaming, redirect 127.0.0.1:5588/login, code exchanged at accounts.spotify.com/api/token, then SpotifyAudio::with_access_token; disconnect removes credentials.json; an audio-key refusal (Error::Auth from open) marks it Degraded until it plays again or signs in). Sources now takes several connectors per service (connectors_for; catalog/source from whichever grants; NotEntitled hint from the connector whose method is meant to grant; streaming_services dedups). services groups connectors by service. spotify.web hint updated. Daemon registers it after spotify.web.

---
▸ 2026-09-26T21:18:42Z [Joel Webber]
Live 2026-09-26, test daemon :7399 over the real state dir: "spotify audio restored for joelgwebber"; services shows spotify.web and spotify.librespot (grants: streaming up to high) both signed in. Prog playlist at volume 0: The Lion's Roar from tidal:36680688 (flac 16/44.1); the Tidal-less tracks from Spotify: "Siberian Khatru - 2003 Remaster" from spotify:1nyLujWRDFnsuKkz1Iq387 (vorbis 44.1 kHz, relinked to the 2025 remaster via alternatives) and "Era" from spotify:7isRVNddDK5D9OcwhuFQoJ; with prefer spotify tidal, The Lion's Roar from spotify:7nsGnc7Wbdywyhi91FmHXL. Preference restored. Not exercised live: the paste-back login (the cached credentials were resumed; URL and state checks are unit-tested) and a real key refusal.

---
▸ 2026-09-26T21:18:44Z [Joel Webber]
verify: `cargo test -p canon-librespot && cargo test -p canon-core source` -> PASS (exit 0)
