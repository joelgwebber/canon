---
id: canon-52fb
title: 'Spike: Spotify audio via librespot as a Source'
type: task
priority: 3
created: '2026-09-24T21:28:12Z'
updated: '2026-09-25T22:54:41Z'
parent: canon-b3a5
labels:
- spotify
- audio
verify: cargo test -p canon-librespot && cargo build -p canon-daemon
---

Step 5. Prototype librespot (OAuth, Premium) opening one Spotify track as a canon Source (Ogg Vorbis decode through the existing engine), nothing wired into playback. Test carefully: audio-key refusals reported since Nov 2025 (librespot #1649). Fallback if it fails: control Connect speakers via the Web API player endpoints (docs/connections.md section 6).

---
▸ 2026-09-24T21:59:07Z [Joel Webber]
Before the librespot spike: is your Spotify account Premium (librespot's audio needs it, and so does owning a dev-mode Web API app)? And OK to sign your account in through librespot (reverse-engineered; Spotify's terms likely forbid it; audio-key refusals reported since Nov 2025)? If not, the fallback is controlling Connect speakers through the Web API player endpoints.

---
▸ 2026-09-25T22:48:07Z [Joel Webber]
Answered in chat 2026-09-25 (transcribed): go ahead with librespot and see how it goes; Joel is signed in to a Spotify Premium account.

---
▸ 2026-09-25T22:54:40Z [Joel Webber]
Built crates/canon-librespot (librespot 0.8.0: core, audio, metadata, oauth): SpotifyAudio::login (librespot OAuth, keymaster client 65b70807..., redirect 127.0.0.1:5588/login, scope streaming; opens the browser) / restore (reusable credentials cached in <state_dir>/spotify.librespot/), open_track -> Track metadata (falling back to relinked alternatives when a track has no files), best file (FLAC 24/16 if offered, else Ogg Vorbis 320/160/96), audio key, AudioFile in stream mode, AudioDecrypt, Spotify Ogg header (0xa7) skipped, wrapped Send+Sync for the decoder -> ResolvedStream (start_ms 0). impl canon_core::Source (open only; describe belongs to spotify.web). canon-audio Symphonia gains ogg + vorbis; Codec::Vorbis hints "ogg". Harness: canon spotify-play <id> [--secs N] (local output, 20% volume). Build note: pinned vergen 9.0.6 (9.1.0 split vergen-lib and broke librespot-core build script).

---
▸ 2026-09-25T22:54:40Z [Joel Webber]
Live 2026-09-25 on Joel Premium account: first run opened the browser, no approval page needed, "Authenticated as joelgwebber", Country US, credentials cached. Track 3z8h0TU7ReDPLIbEnYhWZb first failed "no playable file (offered: [])"; with the alternatives fallback: "spotify: Bohemian Rhapsody as OGG_VORBIS_320", "opened ... as Vorbis in 0.4s", "output at 44100 Hz", played 20 s through canon engine, "stopping after 20s." Second run restored from cache (no browser), offered [AAC_24, OGG_VORBIS_160, OGG_VORBIS_320, OGG_VORBIS_96]: no FLAC for this client. No audio-key refusal seen. Wiring into the daemon, a streaming preference (Tidal first), key-refusal degrade and seeking are filed as canon-c6ac.

---
▸ 2026-09-25T22:54:41Z [Joel Webber]
verify: `cargo test -p canon-librespot && cargo build -p canon-daemon` -> PASS (exit 0)
