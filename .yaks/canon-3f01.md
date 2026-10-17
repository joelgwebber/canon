---
id: canon-3f01
title: Survey codec paths from sources to players to renderers
status: shaving
type: task
priority: 4
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T20:51:18Z'
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
