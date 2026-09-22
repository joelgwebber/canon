---
id: canon-b46b
title: Control-plane API (transport choice + schema)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T13:18:32Z'
parent: canon-1190
labels:
- api
verify: cargo test -p canon-api
---

Expose transport (play/pause/seek/volume), a SERVER-OWNED queue, library browse/search, device pick, and settings, plus the B4 push stream. The queue lives here, not in a client — collapsing tideway's three separate 'now playing' notions into one. Transport choice (gRPC/tonic vs WebSocket+JSON vs REST+SSE) is the open question on this yak.

---
▸ 2026-09-22T02:01:40Z [claude]
Control-plane transport for native UIs (phone/car/TUI) + agents: gRPC/tonic (typed, great for native clients, streaming built-in), WebSocket+JSON (simplest for web/TUI, easy hand-rolled clients), or REST+SSE (what tideway used). This shapes B4 and every client. Any preference or constraint (e.g. must be trivially callable from a vim/TUI)?

---
▸ 2026-09-22T02:36:40Z [Joel Webber]
I think ws+jsonis best for the control plane

---
▸ 2026-09-22T13:17:47Z [Joel Webber]
verify: `cargo test -p canon-api` -> PASS (exit 0)

---
▸ 2026-09-22T13:17:53Z [Joel Webber]
verify: `cargo test -p canon-api` -> PASS (exit 0)

---
▸ 2026-09-22T13:18:11Z [Joel Webber]
Shorn the core deliverable: transport choice (ws+json, your call) + wire schema (protocol.rs) + push stream, verified end-to-end with a real tungstenite client. Carved out: server-owned queue -> canon-23f5. Deferred (additive to the same schema, gated on crates that don't exist yet): library browse/search (canon-4185) and settings (canon-f04a) ops; device-pick already works via select_sink. No playback/auth logic lives in a client.
