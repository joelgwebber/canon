---
id: canon-7531
title: '`canon serve` craps out with crypto panic on startup'
type: bug
priority: 1
created: '2026-10-01T11:55:24Z'
updated: '2026-10-01T12:02:29Z'
verify: cargo build -p canon-daemon
---

```
thread 'main' (10336152) panicked at /Users/joel/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustls-0.23.45/src/crypto/mod.rs:249:14:

Could not automatically determine the process-level CryptoProvider from Rustls crate features.
Call CryptoProvider::install_default() before this point to select a provider manually, or make sure exactly one of the 'aws-lc-rs' and 'ring' features is enabled.
See the documentation of the CryptoProvider type for more information.
```

---
▸ 2026-10-01T12:01:19Z [Joel Webber]
Root cause: two different crypto providers end up linked for rustls 0.23.45 — reqwest 0.13 (canon-spotify/canon-musicbrainz/canon-librespot, direct dep) pulls aws-lc-rs via its rustls feature, while reqwest 0.12 (pulled in transitively through librespot-oauth -> oauth2, a different reqwest major) pulls ring. With both compiled in, rustls process_default_provider() cannot pick automatically and panics on the first TLS handshake. Confirmed by reproducing on the pre-fix binary (git stash): canon serve panics immediately with exactly this trace. Fix: added rustls as a direct canon-daemon dependency (default features, which already select aws-lc-rs, matching what reqwest itself prefers) and call rustls::crypto::aws_lc_rs::default_provider().install_default() as the first line of main(), before Cli::parse() or any networking code can race it. Verified post-fix: canon serve starts clean, discovers the real Chromecast/DLNA renderers (Tunes, Kitchen, Basement speaker, Library display), and canon control can connect and list sinks. Full green bar passes (fmt --check, clippy --workspace --all-targets zero warnings, test --workspace, build --workspace).

---
▸ 2026-10-01T12:01:24Z [Joel Webber]
verify: `cargo build -p canon-daemon` -> PASS (exit 0)

---
▸ 2026-10-01T12:02:29Z [Joel Webber]
Checked whether the same class of bug (ambiguous process-level crypto backend) exists on the Tidal path, as asked: it does not. canon-tidal goes through wreq, which links BoringSSL directly (boring2/boring-sys2/tokio-boring2) -- confirmed via `cargo tree -p canon-tidal -e features`, which shows no rustls crypto backend in that path (only rustls-pki-types, a certificate-type-only crate with no CryptoProvider of its own). BoringSSL has no pluggable-provider step to race, so there is nothing analogous to install_default() needed there. Live-verified canon resolve 33348478 against the real Tidal account post-fix: resolves cleanly, no panic.
