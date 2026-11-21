---
id: canon-5a2c
title: Daemon serves cached, resized album art
status: shaving
type: task
priority: 2
created: '2026-11-21T04:48:21Z'
updated: '2026-11-21T17:30:02Z'
parent: canon-3c5e
tags:
- tui
- api
---

Design decision 1. The daemon fetches artwork_url, caches and resizes it, and serves it over the control plane HTTP side; clients cache by URL and never reach a CDN.

#### 2026-11-21T17:30:02Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving
