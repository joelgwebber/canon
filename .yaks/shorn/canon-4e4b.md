---
id: canon-4e4b
title: 'Build on Linux: drop librespot''s OpenSSL for rustls'
type: bug
priority: 3
created: '2026-09-27T20:26:52Z'
updated: '2026-09-27T20:28:59Z'
labels:
- build
- librespot
needs: human
verify: cargo build --workspace && test -z "$(cargo tree -i openssl-sys --target x86_64-unknown-linux-gnu 2>/dev/null)"
---

On Linux the canon binary fails to link: librespot 0.8's default native-tls pulls openssl-sys (-lssl -lcrypto), while wreq's boring-sys2 puts BoringSSL's libssl.a/libcrypto.a on the link search path first. BoringSSL lacks SSL_read_ex, SSL_write_ex, SSL_CTX_ctrl, SSL_ctrl, SSL_get1_peer_certificate, ERR_get_error_all. macOS never hit it (native-tls uses Security.framework there). Fix: librespot crates with default-features = false + rustls-tls-native-roots.

---
▸ 2026-09-27T20:28:59Z [Joel Webber]
verify: `cargo build --workspace && test -z "$(cargo tree -i openssl-sys --target x86_64-unknown-linux-gnu 2>/dev/null)"` -> PASS (exit 0)

---
▸ 2026-09-27T20:28:59Z [Joel Webber]
Green bar on Linux (CachyOS): fmt ok; clippy --workspace --all-targets 0 warnings; cargo test --workspace all ok (0 failed); cargo build --workspace Finished. cargo tree -i openssl-sys --target x86_64-unknown-linux-gnu: 'package ID specification openssl-sys did not match any packages'.

---
▸ 2026-09-27T20:28:59Z [Joel Webber]
Not shorn: TLS stack under librespot changed (native-tls -> rustls), so it needs a real 'canon spotify-play <id> --secs 20' (sign-in + playback). No librespot credentials on the Linux box, and sign-in needs a browser. Joel: run it once here (or on the Mac, which also switched to rustls), then shear.
