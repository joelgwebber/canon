---
id: canon-e284
title: Playback state core — single source of truth
type: feature
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T21:32:36Z'
labels:
- state
---

The architectural centerpiece. Fixes the class of bug behind tide-2f85: reality (the audio device / active sink) moving without the emitted state moving. Everything that can change playback reality must flow through one state machine that re-emits.
