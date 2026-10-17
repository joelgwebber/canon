---
id: canon-040e
title: Server FLAC stream endpoint for players
status: shaving
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T20:37:28Z'
parent: canon-88a7
source: arch
---

The server resolves a track through `Sources` (Tidal fMP4/FLAC, librespot Ogg Vorbis, local files), decodes it, and serves FLAC at the source's native bit depth and rate over plain HTTP, starting at a requested position. Seek = a new request from the position (renderer-style); no byte ranges yet. The URL is handed to the executor inside the start/prepare effect, carries a short-lived signed token (see the auth yak), and is what a native player would play directly.

The 16-bit `FLAC_BITS` cap stays a renderer rule on the player, not here.

---
▸ 2026-10-17T20:37:28Z [delta:The Great Decoupling]

moved: hairy -> shaving
