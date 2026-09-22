---
id: canon-21f7
title: LAN streaming server (header-replay join + backpressure)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T02:01:39Z'
parent: canon-7718
labels:
- sink
- network
---

The local HTTP server that feeds renderers FLAC. Bind only to the chosen LAN interface. Fix tideway's mid-stream-join bug: cache the FLAC STREAMINFO separately and ALWAYS prepend it to any new/reconnecting consumer (including Cast), then splice onto the live edge at a frame boundary — instead of joining a headerless live ring and playing silence. Per-consumer read cursors over a shared ring; bounded per-reader lag with explicit resync instead of silent oldest-drop. Keep the hard-won interop fixes: force HTTP/1.1 when chunked; never answer 206 without a Range. Crate: axum/hyper.
