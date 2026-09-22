---
id: canon-1190
title: Control API + MCP surface
type: feature
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T13:18:32Z'
parent: canon-dd79
labels:
- api
needs: human
---

The remote surface for UIs (phone/car/TUI), and MCP tools for agents — both thin views over the same core state and command bus, so logic never leaks into a client the way tideway's queue lived in the browser.

---
▸ 2026-09-22T13:18:32Z [Joel Webber]
Control-plane auth/exposure: the ws+json server currently has NO authn/authz on the socket — anyone who can reach the bind addr can drive playback AND trigger a Tidal login. Default bind is 127.0.0.1:7345 (loopback-only), which is safe on one machine but you've said you want native UIs on phone/car and a personally-hosted instance. How do you want to gate remote access? Options: (a) loopback-only + user runs their own TLS-terminating reverse proxy / SSH tunnel; (b) a shared bearer token in the ws handshake; (c) per-client tokens/pairing; (d) mTLS. This shapes the handshake in protocol.rs, so worth deciding before MCP (canon-c67f) and any native client land.
