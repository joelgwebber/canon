---
id: canon-8565
title: Probe connection capabilities and degrade on entitlement failures
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:28:11Z'
parent: canon-b3a5
depends_on:
- canon-c739
labels:
- arch
- tidal
---

Step 2. Probe at login/restore (Tidal: playbackinfo for a known track per quality -> the real Stream ceiling); a 4005/403 during playback downgrades verified capabilities and marks the connection Degraded, surfaced in the connections reply; routing falls through.
