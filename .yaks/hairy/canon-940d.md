---
id: canon-940d
title: cpal local sink + device-loss/sleep-wake recovery
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T02:01:38Z'
parent: canon-b192
labels:
- audio
- sink
---

cpal for enumeration, default-device-change callbacks, exclusive/shared, and the disconnect error path — which is where the DeviceChanged transition (B3) fires. Recovery re-enumerates and reopens (user device, else system default) while the decoder keeps filling the ring for a seamless gap. Register a sleep/wake hook that also rebuilds discovery sockets (shared concern with E2). Resolve the saved device by a stable identity, not a drifting index.
