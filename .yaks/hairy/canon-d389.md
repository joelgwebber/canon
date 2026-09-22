---
id: canon-d389
title: Tidal PKCE flow for hi-res entitlement
type: task
priority: 1
created: '2026-09-22T13:18:11Z'
updated: '2026-09-22T15:31:24Z'
parent: canon-94cc
labels:
- tidal
---

The device-code client (zU4XHVVkc2tDPo4t) is capped at Lossless. Hi-res (HiRes quality) needs Tidal's PKCE auth-code flow with the entitled client, per tideway's tidal_client.py notes. ServiceSession already abstracts login, so PKCE slots in as a second code path in canon-tidal::session behind the same trait. Deferred: device-code login is live and sufficient for Lossless; do PKCE when hi-res is wanted.

---
▸ 2026-09-22T15:31:24Z [Joel Webber]
SCOPE ESCALATION from live testing (see canon-4c55): PKCE is not just for hi-res — the device-code client can no longer stream ANY quality (Tidal returns 4005 on all playbackinfo/urlpostpaywall). PKCE (client 6BDSRdpK9hqEBTgU, auth-code + code_challenge, browser redirect capture) is now REQUIRED for ALL Tidal playback. This blocks the Tidal half of the audio pipeline. Priority raised.
