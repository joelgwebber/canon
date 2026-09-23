---
id: canon-c67f
title: MCP tool layer over the core
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-23T14:51:15Z'
depends_on:
- canon-fdf3
labels:
- api
- mcp
---

One MCP tool per verb — play, pause, seek, enqueue, search, resolve_track, add_to_playlist, import_playlist, pick_device — each calling the same core command bus as the API, so 'ask my agent to play things and curate my library' is a first-class path. Crate: rmcp (official Rust MCP SDK).
