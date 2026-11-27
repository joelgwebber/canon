---
id: canon-1fb0
title: 'TUI pass 3, step 1: one protocol behind every interface'
status: shaving
type: task
priority: 2
created: '2026-11-27T00:25:43Z'
updated: '2026-11-27T00:31:32Z'
parent: canon-3c5e
tags:
- api
---

[The third pass](/doks/design/tui-places.md), steps 1 (and Decisions 7). Streamed replies (more: true; the last says done), a track detail op, service links in views (built by the daemon), and a test that canon control reaches every op. Gathers canon-2b75.

#### 2026-11-27T00:27:43Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-27T00:31:32Z

*herd-player*

Batch (herd-player), base canon 5572553, both on claude-sonnet-5-5: proto-stream → canon-9cc1 (streamed replies, search everywhere; test daemon 7397/7349), proto-detail → canon-bfe6 (track detail op, service links, control coverage; 7398/7348). Shared files: message.rs and view.rs (each adds its own variants/fields; a textual merge expected).
