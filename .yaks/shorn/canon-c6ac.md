---
id: canon-c6ac
title: Spotify audio as a connection (spotify.librespot), behind a streaming preference
type: task
priority: 3
created: '2026-09-25T22:54:14Z'
updated: '2026-09-26T21:23:12Z'
parent: canon-b3a5
labels:
- spotify
- audio
verify: cargo test -p canon-librespot && cargo test -p canon-library && cargo test -p canon-core source
---

Follow-up to the canon-52fb spike (SpotifyAudio in crates/canon-librespot works live: sign-in, Ogg Vorbis 320, canon's engine decodes it). To wire it: (1) a SpotifyLibrespotConnector with method spotify.librespot (Stream at High: Vorbis 320; Spotify offered no FLAC to this client), credentials cached in <state_dir>/spotify.librespot/. Its login cannot use librespot-oauth's get_access_token_async inside the daemon as is: it blocks on a std TcpListener accept on 127.0.0.1:5588 inside an async fn (it would stall a tokio worker, and the port collides with nothing but still needs the browser on the same machine); build the PKCE authorize URL ourselves (client 65b708073fc0480ea92a077233ca87bd, redirect http://127.0.0.1:5588/login, scope streaming) and complete by paste-back like tidal.pkce, then Credentials::with_access_token. (2) A streaming PREFERENCE (setting, default Tidal first): Joel wants Tidal for audio, so queue-time matching (Library::playable) must match onto the preferred service even when a track has a streamable Spotify binding, and use Spotify audio only when the preferred service has no match. Today can_stream(spotify) would skip matching. (3) Audio-key refusal -> Error::Auth -> degrade the connection, like Tidal's Observed. (4) Seeking: the stream always starts at 0 (start_ms 0 keeps the clock honest, but a seek restarts the track); AudioDecrypt is seekable, so opening at a position needs the engine to accept a seekable input (see canon-6a92) or a byte-offset estimate from the bitrate.

---
▸ 2026-09-26T19:54:49Z [Joel Webber]
Step (2), the streaming preference, is done as canon-5496. Remaining here: the spotify.librespot connector with a paste-back login, audio-key refusal -> degrade, and seeking.

---
▸ 2026-09-26T21:23:03Z [Joel Webber]
Done through its children: canon-5496 (streaming preference + remembered misses), canon-fa18 (playing_from logged and in the snapshot), canon-58c3 (spotify.librespot connector, several connectors per service), canon-5305 (seeking via seek_to). Live: Tidal-less Prog tracks play from Spotify Vorbis, the rest from Tidal FLAC; prefer spotify tidal flips it; seeks land within Spotify tracks.

---
▸ 2026-09-26T21:23:12Z [Joel Webber]
verify: `cargo test -p canon-librespot && cargo test -p canon-library && cargo test -p canon-core source` -> PASS (exit 0)
