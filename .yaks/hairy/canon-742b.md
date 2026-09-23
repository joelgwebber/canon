---
id: canon-742b
title: Ship a signed macOS bundle (firewall + Local Network friendly)
type: task
priority: 3
created: '2026-09-23T01:54:26Z'
updated: '2026-09-23T03:27:26Z'
parent: canon-f495
labels:
- sink
- network
- macos
---

On macOS the Application Firewall auto-allows only SIGNED software, so canon's LAN listener (the stream server renderers fetch from) is silently blocked when run as an unsigned/ad-hoc cargo binary — inbound TCP is reset while loopback works. Ad-hoc/linker-signed binaries get a fresh code identity every build, so neither the firewall allow-list nor a TCC grant can durably match them. Dev workaround: sudo socketfilterfw --unblockapp target/debug/canon (stable path, survives rebuilds). Proper fix for distribution: sign (and for a .app, notarize) the binary with a stable identity, add the Local Network usage string + multicast entitlement for discovery. No signing identity exists on the dev machine today (security find-identity = 0). Unaffected on Linux/headless, which is the intended deployment target. Related: canon-bb20 (mDNS via system DNS-SD).
