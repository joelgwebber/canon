---
id: canon-2b75
title: Revisit API/CLI coherence in light of UI restructuring
status: shorn
type: idea
priority: 3
created: '2026-11-21T19:15:45Z'
updated: '2026-11-27T01:08:02Z'
parent: canon-1fb0
tags:
- api
- cli
---

With all the changes we're making to the TUI, let's take a moment to revisit the implications for the API/CLI layers, and ensure both that the API supports the UI affordances we're building, as well as (eventually) agent-friendly affordances via MCP or CLI (canon-c67f). We don't want any human/agent interface to be more expressive than another.

#### 2026-11-27T00:26:11Z

*herd-player*

Designed in [the third pass](/doks/design/tui-places.md) with Joel, 2026-11-21; built under its step yak (parent).

#### 2026-11-27T01:08:01Z

*herd-player*

Built by step 1 (canon-1fb0): one protocol behind every interface. canon-bfe6 added a test that fails when a ClientMessage op has no canon control command (CONTROL_OPS/NOT_IN_CONTROL, each exception argued), plus track detail and links reachable from control; canon-9cc1 made search everywhere the server's, so control and MCP get it too. Landed as canon 22683f6, afbc411, f800a39 (rebased onto Joel's 1564aa8).

#### 2026-11-27T01:08:02Z

*herd-player · kind: move · from: hairy · to: shorn · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shorn
