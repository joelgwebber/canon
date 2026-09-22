---
id: canon-21f7
title: LAN streaming server (header-replay join + backpressure)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T22:44:43Z'
parent: canon-7718
labels:
- sink
- network
verify: cargo test -p canon-sink
---

The local HTTP server that feeds renderers FLAC. Bind only to the chosen LAN interface. Fix tideway's mid-stream-join bug: cache the FLAC STREAMINFO separately and ALWAYS prepend it to any new/reconnecting consumer (including Cast), then splice onto the live edge at a frame boundary — instead of joining a headerless live ring and playing silence. Per-consumer read cursors over a shared ring; bounded per-reader lag with explicit resync instead of silent oldest-drop. Keep the hard-won interop fixes: force HTTP/1.1 when chunked; never answer 206 without a Range. Crate: axum/hyper.

---
▸ 2026-09-22T22:44:30Z [Joel Webber]
verify: `cargo test -p canon-sink` -> PASS (exit 0)

---
▸ 2026-09-22T22:44:43Z [Joel Webber]
Shorn. Landed crates/canon-sink/src/stream_server.rs (built by a subagent, reviewed + integrated by me).
Two decoupled layers:
- StreamBroadcaster (protocol-agnostic core): header cached SEPARATELY and always replayed first to every subscriber (incl. a Cast reconnect); live edge is a bounded broadcast a joiner enters at the current tail (never the whole history). This is the tideway headerless-live-ring fix, structural not a retry.
- Backpressure = resync-by-disconnect: on broadcast Lagged the subscriber stream ENDS (renderer reconnects, replays header, rejoins) rather than a silent mid-FLAC oldest-drop.
- axum layer (router/serve/spawn): 200 audio/flac chunked; binds ONLY the chosen iface (explicit SocketAddr, never 0.0.0.0); HTTP/1.1 by default on plain TCP; 206 structurally quarantined behind an if-Range guard (never emitted without a Range) as the DLNA-seek extension point.
PCM->FLAC encoding intentionally out of scope (wired in canon-dde4); this deals in already-encoded header+frame Bytes.
Note: tokio-stream's BroadcastStream wrapper needs its 'sync' feature (not enabled), so LiveEdge is hand-rolled over broadcast::Receiver — correct + Send, allocates a small recv future per chunk (network side, not the RT path; negligible). Enable tokio-stream/sync later if the stock wrapper is preferred.
Evidence: cargo test -p canon-sink (7 stream_server tests incl. header-replay, late-joiner edge-only, lag->end, 200+audio/flac, no-206-without-Range, real-port bind); workspace fmt clean, clippy 0 warnings, all tests green.
