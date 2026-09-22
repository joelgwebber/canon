---
id: canon-dde4
title: Chromecast control + state feedback
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T02:01:39Z'
parent: canon-7718
labels:
- sink
- network
---

Connect + play_media(url, audio/flac, LIVE) + track-change re-issue, via the Cast app framework (crate: rust_cast). CLOSE THE FEEDBACK LOOP tideway left open: route MediaStatus (player_state, idle_reason, external takeover) back INTO the state machine (B) as an input, so a phone hijacking the session surfaces instead of the app asserting stale control.
