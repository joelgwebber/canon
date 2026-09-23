---
id: canon-1190
title: Control API
type: feature
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-23T04:25:46Z'
labels:
- api
---

The remote surface for UIs (phone/car/TUI), and MCP tools for agents — both thin views over the same core state and command bus, so logic never leaks into a client the way tideway's queue lived in the browser.

---
▸ 2026-09-22T13:18:32Z [Joel Webber]
Control-plane auth/exposure: the ws+json server currently has NO authn/authz on the socket — anyone who can reach the bind addr can drive playback AND trigger a Tidal login. Default bind is 127.0.0.1:7345 (loopback-only), which is safe on one machine but you've said you want native UIs on phone/car and a personally-hosted instance. How do you want to gate remote access? Options: (a) loopback-only + user runs their own TLS-terminating reverse proxy / SSH tunnel; (b) a shared bearer token in the ws handshake; (c) per-client tokens/pairing; (d) mTLS. This shapes the handshake in protocol.rs, so worth deciding before MCP (canon-c67f) and any native client land.

---
▸ 2026-09-22T14:00:05Z [Joel Webber]
Yeah, let's keep the clients as thin as possible for the time being; I'm imagining just running it on a local network.

---
▸ 2026-09-22T14:01:44Z [Joel Webber]
DECISION (Joel): no authn/authz on the control-plane socket for now — thin clients on a trusted LAN. Keep the default bind loopback (127.0.0.1:7345) as the safe default; LAN exposure is an explicit opt-in via `canon serve --bind 0.0.0.0:7345`. protocol.rs handshake stays token-free; revisit (bearer token / pairing) before any untrusted-network or hosted deployment. Unblocks MCP (canon-c67f) and a first native/TUI client with no auth surface to build yet.

---
▸ 2026-09-23T04:25:21Z [Joel Webber]
Hoisting canon-c67f out for later.
