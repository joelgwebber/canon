---
id: canon-1175
title: Investigate Spotify as a canon source (library + audio) — feasibility only
type: task
priority: 3
created: '2026-09-22T02:47:45Z'
updated: '2026-09-22T15:39:19Z'
parent: canon-dd79
labels:
- spotify
- research
---

RESEARCH ONLY. Do not implement until/unless we decide to; parked as a known-feasible future source. Question: is there a public API or a proven workaround to access Spotify libraries AND music the way canon does for Tidal? Answer: yes on both halves, with one hard constraint (Premium).

LIBRARY + METADATA (official, supported): the Spotify Web API (OAuth Authorization Code + PKCE) gives saved tracks/albums, playlists, search, and catalog metadata. tideway already proves this half — spotify_import.py does the OAuth-PKCE import, spotify_public.py scrapes anonymous GraphQL for playcounts. CAVEAT: in Nov 2024 Spotify deprecated several Web API endpoints for new apps (audio-features, audio-analysis, recommendations, related-artists, featured/category playlists, 30s preview URLs), so metadata enrichment is thinner than it was, but core library/playlist access is intact.

AUDIO (not via official API): the Web API deliberately exposes NO raw/decrypted stream. Officially you can only (a) control playback on official Spotify clients via the Connect API, or (b) use the Web Playback SDK in a browser (Widevine DRM) — both Premium, neither gives us PCM/bytes.

EXISTENCE PROOF of the workaround: librespot (github.com/librespot-org/librespot) — a mature, actively-maintained, pure-Rust (MIT) reverse-engineered Spotify client. It authenticates as a Spotify Connect device and pulls + decrypts Ogg Vorbis audio; its crates already split out core/oauth/metadata/playback/protocol/discovery. It powers spotifyd, ncspot, Snapcast, Mopidy, Music Assistant, etc. This is the exact analog of what canon's Tidal source does, and conveniently it's already Rust, so it could slot behind canon-core's Source trait with modest glue.

HARD CONSTRAINT: librespot is Premium-only, permanently and by policy ('We will not support any features to make librespot compatible with free accounts'). So a Spotify audio source requires a Premium account; free accounts are a non-starter for streaming (library/metadata import could still work on any account).

ToS/risk posture: same as the Tidal decision on canon-94cc — librespot's own README says connecting this way is 'probably forbidden by them. Use at your own risk.' Treat a Spotify source as the same personal-use, degrade-honestly posture, and likely off-by-default.

SHAPE IF WE EVER BUILD IT: a SpotifySource = official Web API for library/browse/metadata + librespot for audio (Premium-gated), implementing the same Source trait as Tidal. Also validates that the trait's seams (auth lifecycle, resolve-to-stream, MediaSource) generalise beyond Tidal — which is the whole point of the service-agnostic library (F). Next step when picked up: prototype librespot audio fetch for one track + Web API saved-tracks pull, behind the Source trait, nothing wired into playback yet.
