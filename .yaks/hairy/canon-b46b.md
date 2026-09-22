---
id: canon-b46b
title: Control-plane API (transport choice + schema)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T02:36:40Z'
parent: canon-1190
labels:
- api
---

Expose transport (play/pause/seek/volume), a SERVER-OWNED queue, library browse/search, device pick, and settings, plus the B4 push stream. The queue lives here, not in a client — collapsing tideway's three separate 'now playing' notions into one. Transport choice (gRPC/tonic vs WebSocket+JSON vs REST+SSE) is the open question on this yak.

---
▸ 2026-09-22T02:01:40Z [claude]
Control-plane transport for native UIs (phone/car/TUI) + agents: gRPC/tonic (typed, great for native clients, streaming built-in), WebSocket+JSON (simplest for web/TUI, easy hand-rolled clients), or REST+SSE (what tideway used). This shapes B4 and every client. Any preference or constraint (e.g. must be trivially callable from a vim/TUI)?

---
▸ 2026-09-22T02:36:40Z [Joel Webber]
I think ws+jsonis best for the control plane
