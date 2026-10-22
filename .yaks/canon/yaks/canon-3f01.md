---
id: canon-3f01
title: Survey codec paths from sources to players to renderers
status: shorn
type: task
priority: 4
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T20:56:29Z'
parent: canon-88a7
source: arch
---

Not blocking the decoupling: decided there is FLAC everywhere for now. Map what actually flows, check it against real devices, and say whether we leave real cost on the table.

Starting point, from memory, to verify:
- Sources: Tidal FLAC in fMP4 (16/44.1 to 24/192; AAC at low tiers); librespot Ogg Vorbis 96/160/320 (is Spotify lossless reachable?); local files, anything.
- Outputs: Cast FLAC to ~24/96 plus AAC/MP3/Opus/Vorbis; DLNA varies (we cap 16-bit, device-quirks.md); Sonos FLAC to 24/48; AirPlay v1 ALAC 16/44.1 over RTP, v2 adds AAC and pairing (owntone, pyatv); iOS no Ogg Vorbis/Opus; Android nearly everything.
- Known costs: lossy Vorbis re-sent as FLAC is ~3x the bytes for nothing (matters on cell: Ogg passthrough or the lossy tier); 24/192 is 4-5 Mbps; renderers above 96k need resampling (keep it on the player). CPU for FLAC decode and re-encode is a few percent of a core.

---
▸ 2026-10-17T20:51:18Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T20:56:29Z [delta:The Great Decoupling]

Survey (scout subthread, claude-haiku-5-5; read-only, sources linked in its report). Summary:

- Tidal: Corrected. Consumer tiers: Max = FLAC up to 24/192, High = FLAC 16/44.1, Low = AAC to 320. API enum (SDK): LOW AAC 96, HIGH AAC 320, LOSSLESS FLAC 16/44.1, HI_RES_LOSSLESS FLAC. Container fMP4/DASH per our own stream.rs; Tidal doesn't document it.
- Spotify: Corrected. Lossless (Sep 2025) is 24/44.1 FLAC, not hi-res, and librespot can't reach it (librespot#1583 locked; #1589 adds FLAC decode only). Vorbis 96/160/320 confirmed.
- Chromecast: Verified. FLAC up to 24/96 on Cast audio devices; also AAC, MP3, Opus, Vorbis, WAV. No published limit for the KEF.
- DLNA: Corrected. device-quirks.md says nothing about depth; the 16-bit cap is an unsourced constant (`FLAC_BITS`); DLNA's mandatory profile is 16-bit LPCM, FLAC optional.
- Sonos: FLAC/ALAC to 24/48 (S2, local library); 16-bit for streaming services. Not a canon output.
- AirPlay: Verified. v1 ALAC 16/44.1; v2 realtime same, buffered adds AAC and ALAC 24/48. shairport-sync is the mature receiver; owntone lacks buffered (24-bit) sending.
- Native: iOS plays FLAC (11+), ALAC, AAC; no Ogg Vorbis; Opus only in CAF. Android Media3 plays FLAC/Ogg/Opus; platform FLAC is ≤48 kHz, so hi-res needs the software FLAC extension.
- Bitrates: FLAC ≈ 0.7–1.0 Mbps (16/44.1), 2.3–3.2 (24/96), 4.6–6.5 (24/192); Vorbis 320 = 0.32.

Where canon leaves real cost, ranked:
1. Tidal hi-res never plays: every player's controller opens at `Quality::Lossless` (main.rs, players::Shared) though the PKCE login grants HiRes. Filed as its own yak.
2. Network renderers capped at 16-bit by rounding, not dither (only bites once hi-res is requested).
3. A 192 kHz track would go to Cast above its 96 kHz limit (the tap keeps the source rate).
4. Spotify tops out at Vorbis 320 via librespot, no legal route to lossless.

Conclusion: FLAC everywhere stands; no change needed before the first native player beyond (1), and making sure native serving doesn't inherit FLAC_BITS (canon-040e serves native depth: confirmed). Android needs the software FLAC extension for hi-res. Ogg passthrough to save cell bandwidth is real but small next to (1); left as an idea, not filed.

---
▸ 2026-10-17T20:56:29Z [delta:The Great Decoupling]

moved: shaving -> shorn
