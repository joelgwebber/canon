---
id: canon-d389
title: Tidal PKCE flow for hi-res entitlement
type: task
priority: 2
created: '2026-09-22T13:18:11Z'
updated: '2026-09-22T13:18:11Z'
parent: canon-94cc
labels:
- tidal
---

The device-code client (zU4XHVVkc2tDPo4t) is capped at Lossless. Hi-res (HiRes quality) needs Tidal's PKCE auth-code flow with the entitled client, per tideway's tidal_client.py notes. ServiceSession already abstracts login, so PKCE slots in as a second code path in canon-tidal::session behind the same trait. Deferred: device-code login is live and sufficient for Lossless; do PKCE when hi-res is wanted.
