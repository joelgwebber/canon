---
id: canon-a880
title: HTTP + TLS-fingerprint impersonation layer
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T03:33:29Z'
parent: canon-94cc
labels:
- tidal
- network
verify: cargo test -p canon-tidal
---

Tidal's anti-abuse blocks default HTTP stacks by TLS/HTTP2 fingerprint (tideway impersonates chrome-android via curl-cffi). Canon needs the same, but the pure-Rust option is unstable: rquest's recent versions are YANKED (the reqwest-impersonate -> rquest -> wreq lineage churns). So isolate ALL Tidal HTTP behind a small trait (get/post/ws) and treat the impersonation backend as swappable. See the open question on this yak. Also carry tideway's module-wide 429/abuse backoff gate to avoid escalating to a ban.

---
▸ 2026-09-22T02:01:40Z [claude]
TLS-fingerprint impersonation backend: (a) pure-Rust wreq (rquest successor; keeps the single-static-binary goal but the crate lineage churns and recent rquest versions are yanked), or (b) vendor/link curl-impersonate via FFI (matches tideway's curl-cffi exactly, most robust, but adds a C dependency). Either way it sits behind a trait. Which do you want as the default, and is a C dependency acceptable given the 'simple self-contained binary' goal?

---
▸ 2026-09-22T02:38:50Z [Joel Webber]
Let's do a quick spike to see if the pure rust wreq can work properly; we can accept the C FFI if needed.

---
▸ 2026-09-22T03:33:29Z [Joel Webber]
verify: `cargo test -p canon-tidal` -> PASS (exit 0)

---
▸ 2026-09-22T03:33:29Z [claude]
RESOLVED via spike. Pure-Rust wreq is viable: wreq 0.15.3 + wreq-util 0.1 build cleanly here (vendored BoringSSL ~24s; cmake+clang present) and present a genuine Chrome-136-on-Android fingerprint (JA3 d06498e4..., JA4 t13d1516h2_8daaf6152771..., Chrome H2 SETTINGS + m,a,s,p pseudo-header order), verified LIVE against tls.peet.ws. Built the swappable seam: canon_core-error-integrated TidalHttp trait (get/post_form + HttpResponse::json) with a WreqHttp backend behind it, so a curl-impersonate C-FFI impl can drop in later without touching callers -- fallback stays UNBUILT. Two caveats carried forward: (1) wreq 0.16.x needs rustc 1.98 (we pin 1.95), so we're on the 0.15 line; revisit on a toolchain bump. (2) fingerprint proven against a reflector, NOT Tidal end-to-end (no creds); that validation belongs to the auth impl. Evidence: cargo test -p canon-tidal (4 pass, live fingerprint test ignored-by-default), clippy -D warnings clean.
