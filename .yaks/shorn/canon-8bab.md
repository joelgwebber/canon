---
id: canon-8bab
title: Tidal auth + token lifecycle (rotating refresh, single-flight)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T14:15:53Z'
parent: canon-94cc
labels:
- tidal
verify: cargo test -p canon-tidal
---

Two OAuth flows: device-code (caps at Lossless) and PKCE (unlocks Max/hi-res); the quality ceiling is gated by client-id, not subscription, so persist an is_pkce flag and re-enable hi-res on load. CRITICAL: Tidal rotates refresh tokens — capture the new refresh_token from every refresh response or the session silently dies days later. Single-flight refresh under one lock (watchdog + on-401 share it). Classify invalid_grant/invalid_client as permanent vs 429/5xx as transient; conflating them was tideway's silent-logout bug.

---
▸ 2026-09-22T03:33:31Z [claude]
Spike left a COMPILE-ONLY device-code OAuth skeleton (canon-tidal/src/auth.rs): typed request/response structs + the start_device_authorization -> poll_device_token -> refresh_token flow over &dyn TidalHttp, client id zU4XHVVkc2tDPo4t against auth.tidal.com, with the ROTATING refresh token modelled explicitly (Tokens::apply captures a rotated refresh_token, preserves the old otherwise) and unit-tested. NOT a live login (no creds attempted). Still TODO for the real lifecycle: PKCE flow (for Max/hi-res), single-flight refresh under one lock shared by watchdog + on-401, proactive pre-expiry refresh watchdog, permanent-vs-transient failure classification, and 0600 token persistence. Leaving shaving.

---
▸ 2026-09-22T13:17:50Z [Joel Webber]
verify: `cargo test -p canon-tidal` -> PASS (exit 0)

---
▸ 2026-09-22T13:17:54Z [Joel Webber]
verify: `cargo test -p canon-tidal` -> PASS (exit 0)

---
▸ 2026-09-22T13:18:11Z [Joel Webber]
Shorn: live device-code login end to end. TidalSession implements ServiceSession; rotating refresh captured via Tokens::apply; single-flight refresh (refresh runs under the same lock the token is read from); atomic token persistence so restart resumes. Verified against LIVE Tidal: canon login tidal returned real device code EPKNS and polled; auth/session/account paths mock-tested green. Found+fixed via live call: device_authorization is camelCase. Follow-up spun out: PKCE for hi-res (see new yak).

---
▸ 2026-09-22T14:15:53Z [Joel Webber]
Live end-to-end proof (regrown to fold in the fix the live run surfaced): 'canon login tidal' authenticated as real user 189763387 (US); tokens persisted; 'canon serve' restored the session from disk and an {op:account} over the ws control plane returned user_id/country_code/session_id. Bug found+fixed via the live run: the device-code TOKEN endpoint needs the TV client's client_secret (+ scope resent on the poll), else Tidal issues a scopeless token that 403s ('Token is missing required scope'). Mock-HTTP unit tests still green.

---
▸ 2026-09-22T14:15:53Z [Joel Webber]
verify: `cargo test -p canon-tidal` -> PASS (exit 0)
