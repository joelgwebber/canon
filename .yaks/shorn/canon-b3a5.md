---
id: canon-b3a5
title: 'Connections: per-login capabilities for every service'
type: feature
priority: 2
created: '2026-09-24T21:27:55Z'
updated: '2026-09-26T21:23:15Z'
labels:
- arch
verify: cargo test --workspace
---

Build the design in docs/connections.md (agreed 2026-09-24, from canon-699d): connections named by (service, method) with declared and verified capabilities, connectors that describe their login methods as data, routing by capability, typed NotEntitled errors. One account per service per instance.

---
▸ 2026-09-26T21:23:12Z [Joel Webber]
Done: docs/connections.md steps 1-5 built (canon-c739 connections/Tidal, canon-8565 probing, canon-8b07 + canon-6272 Spotify Web API, canon-52fb + canon-c6ac Spotify audio), with ISRC matching (canon-b391, canon-4054) under canon-880e. Later options stay in the doc: official Tidal v2 for library access, canon as a Spotify Connect receiver.

---
▸ 2026-09-26T21:23:15Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)
