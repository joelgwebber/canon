---
id: canon-d419
title: Drop the cmake/aws-lc-sys C build dep from rust_cast TLS (move rustls to ring)
type: chore
priority: 3
created: '2026-09-23T00:23:45Z'
updated: '2026-09-23T00:23:45Z'
labels:
- sink
---

rust_cast 0.21 pulls rustls with its default provider aws-lc-rs -> aws-lc-sys (bundled C, built via cmake). Runtime is still a self-contained static binary, but the BUILD needs cmake + a C compiler (slow, heavy) — it will land in CI installer builds. Feature unification is additive so it can't be stripped transitively; fixing it needs rust_cast on rustls default-features=false with a lighter provider. TARGET: ring (Rust + asm/C via cc, NO cmake) — user is fine with embedded asm/C, just not cmake. Fully-pure alternative: rustls-rustcrypto (pure Rust, unaudited; fine for a self-signed LAN Cast cert). Approach: [patch.crates-io] a small rust_cast fork (rustls dep tweak) or upstream a provider feature. Not urgent; canon-dde4 ships on aws-lc-sys first.
