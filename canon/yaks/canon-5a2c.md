---
id: canon-5a2c
title: Daemon serves cached, resized album art
status: hairy
type: task
priority: 2
created: '2026-11-21T04:48:21Z'
updated: '2026-11-21T04:48:21Z'
parent: canon-3c5e
tags:
- tui
- api
---

Design decision 1. The daemon fetches artwork_url, caches and resizes it, and serves it over the control plane HTTP side; clients cache by URL and never reach a CDN.
