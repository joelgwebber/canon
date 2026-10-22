---
id: canon-934b
title: Play Tidal hi-res when the login allows it
status: hairy
type: task
priority: 2
created: '2026-10-17T20:56:29Z'
updated: '2026-10-17T20:56:29Z'
parent: canon-88a7
source: arch
---

Found by the codec survey (canon-3f01). Every player's controller opens tracks at `Quality::Lossless` (`players::Shared.quality`, set in `run_serve`), so a PKCE login that grants HiRes never plays hi-res; only the startup probe asks for it. The server's /audio endpoint already asks for `Quality::HiRes` and serves native depth.

Decide and do: the quality a player streams at (probably what the login grants, capped by a per-player setting, the same setting the lossy tier will use). When feeding renderers, resample anything above 96 kHz before Cast (Google's documented limit), and decide whether the 16-bit renderer cap should dither rather than round.
