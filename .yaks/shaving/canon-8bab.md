---
id: canon-8bab
title: Tidal auth + token lifecycle (rotating refresh, single-flight)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T03:33:31Z'
parent: canon-94cc
labels:
- tidal
---

Two OAuth flows: device-code (caps at Lossless) and PKCE (unlocks Max/hi-res); the quality ceiling is gated by client-id, not subscription, so persist an is_pkce flag and re-enable hi-res on load. CRITICAL: Tidal rotates refresh tokens — capture the new refresh_token from every refresh response or the session silently dies days later. Single-flight refresh under one lock (watchdog + on-401 share it). Classify invalid_grant/invalid_client as permanent vs 429/5xx as transient; conflating them was tideway's silent-logout bug.

---
▸ 2026-09-22T03:33:31Z [claude]
Spike left a COMPILE-ONLY device-code OAuth skeleton (canon-tidal/src/auth.rs): typed request/response structs + the start_device_authorization -> poll_device_token -> refresh_token flow over &dyn TidalHttp, client id zU4XHVVkc2tDPo4t against auth.tidal.com, with the ROTATING refresh token modelled explicitly (Tokens::apply captures a rotated refresh_token, preserves the old otherwise) and unit-tested. NOT a live login (no creds attempted). Still TODO for the real lifecycle: PKCE flow (for Max/hi-res), single-flight refresh under one lock shared by watchdog + on-401, proactive pre-expiry refresh watchdog, permanent-vs-transient failure classification, and 0600 token persistence. Leaving shaving.
