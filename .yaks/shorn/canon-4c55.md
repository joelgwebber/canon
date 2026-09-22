---
id: canon-4c55
title: 'Stream resolution: MPD → segment list + manifest cache'
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T16:31:24Z'
parent: canon-94cc
depends_on:
- canon-d389
labels:
- tidal
verify: cargo test -p canon-tidal
---

playbackinfo returns a base64 DASH/MPD manifest. Do our OWN MPD → segment-URL extraction (init at index 0, media 1..N fragmented-MP4); do NOT feed Tidal's MPD to a generic DASH demuxer (libav rejects it across tiers). Reject is_encrypted manifests. Cache resolved manifests by (track_id, quality) with a ~180s TTL sized to the signed-URL lifetime. Crates: quick-xml or dash-mpd.

---
▸ 2026-09-22T15:31:24Z [Joel Webber]
Stream-resolution code is implemented and unit-tested (BTS + DASH parse, sessionId/countryCode/x-tidal-client-version + Tidal app UA, impersonated). BUT playback is BLOCKED by a Tidal-side gate: the device-code client zU4XHVVkc2tDPo4t (cid 3235) authenticates fine (/v1/sessions OK) but playbackinfopostpaywall + urlpostpaywall return 401 subStatus 4005 'Asset is not ready for playback' for EVERY track/quality, even over tideway's own proven curl_cffi transport + Tidal app UA. Verified against a PREMIUM/HI_RES account with streamReady=true tracks. Refreshing tideway's own stored token (cid 8017) under the TV client also yields cid 3235 and also 4005. Conclusion: device-code tokens can no longer stream; playback requires the PKCE mobile client (6BDSRdpK9hqEBTgU). See canon-d389.

---
▸ 2026-09-22T16:30:45Z [Joel Webber]
verify: `cargo test -p canon-tidal` -> PASS (exit 0)

---
▸ 2026-09-22T16:31:24Z [Joel Webber]
Shorn: unblocked by PKCE (canon-d389) and verified end to end against the live service. canon play 520285418 resolved a real DASH manifest (63 segments, FLAC 44.1k/16), fetched ~30MB, and played. Two live bugs fixed along the way: playback requires sessionId+countryCode+x-tidal-client-version+Tidal-app-UA (was 4005), and DASH segment URLs must be XML-unescaped (&amp; -> &, was 403).
