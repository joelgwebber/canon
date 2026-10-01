---
id: canon-775e
title: Error connecting to chromecast device on linux
type: bug
priority: 1
created: '2026-10-01T03:57:00Z'
updated: '2026-10-02T03:12:21Z'
labels:
- cast
- linux
verify: cargo build -p canon-daemon && grep -q "aws_lc_rs::default_provider()" crates/canon-daemon/src/main.rs
---

When connecting to my KEF "Tunes" device over cast:

```
thread 'tokio-rt-worker' (251500) panicked at /home/joel/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rustls-0.23.45/src/crypto/mod.rs:249:14:

Could not automatically determine the process-level CryptoProvider from Rustls crate features.
Call CryptoProvider::install_default() before this point to select a provider manually, or make sure exactly one of the 'aws-lc-rs' and 'ring' features is enabled.
See the documentation of the CryptoProvider type for more information.
```

---
▸ 2026-10-01T11:33:42Z [Joel Webber]
Cause: canon-4e4b moved librespot to rustls, and its hyper-rustls enables rustls/ring, while rust_cast's rustls (default features) enables aws-lc-rs. With both providers compiled in, rustls can't choose a process default, so the first ClientConfig::builder() (rust_cast's Cast TLS channel) panics. Not Linux-specific: the features aren't target-gated, so a fresh macOS build would panic the same way. Fix: canon's main() installs ring as the process default before any TLS (ring because canon-d419 means to drop aws-lc-rs/cmake), so feature unification no longer decides it.

---
▸ 2026-10-01T11:33:42Z [Joel Webber]
Live 2026-10-01 (Linux, test daemon :7399): sinks listed Tunes over chromecast; 'sink Tunes@cast' + enqueue 33348478 -> no panic; the Cast channel connected and the speaker took the LOAD. Playback then failed for a different reason: 'renderer session lost: ... It never fetched a stream from http://192.168.0.27:38705'. journalctl -k shows '[UFW BLOCK] SRC=192.168.0.205 DST=192.168.0.27 PROTO=TCP DPT=38705': this machine's ufw (default input DROP) blocked the speaker's fetch. That is the firewall, tracked on canon-6227, not this panic. End-to-end cast playback on Linux is therefore NOT yet verified. Green bar: fmt, clippy 0 warnings, test all ok, build.

---
▸ 2026-10-01T11:33:43Z [Joel Webber]
verify: `cargo build -p canon-daemon && cargo tree -i rustls@0.23.45 -e features | grep -q "rustls feature \"aws_lc_rs\"" && grep -q "ring::default_provider().install_default()" crates/canon-daemon/src/main.rs` -> PASS (exit 0)

---
▸ 2026-10-02T03:12:21Z [Joel Webber]
Rebase 2026-10-01: canon-7531 (5d1d4eb) landed the same fix first, installing aws-lc-rs (reqwest 0.13's own choice) with .expect() as main()'s first line. This yak's ring install was dropped in favour of it: kept alongside, the two rustls lines collided in canon-daemon/Cargo.toml, and the second install would only have failed silently. Re-checked on the resolved tree (Linux, :7399): daemon starts (librespot restore = canon-7531's panic site), 'sink Tunes@cast' + enqueue -> Cast channel connected and LOAD taken, 0 panics in the logs; then only the ufw 'never fetched' (canon-6227). Green bar ok.
