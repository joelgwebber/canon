---
id: canon-940d
title: cpal local sink + device-loss/sleep-wake recovery
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T17:26:34Z'
parent: canon-b192
labels:
- audio
- sink
---

cpal for enumeration, default-device-change callbacks, exclusive/shared, and the disconnect error path — which is where the DeviceChanged transition (B3) fires. Recovery re-enumerates and reopens (user device, else system default) while the decoder keeps filling the ring for a seamless gap. Register a sleep/wake hook that also rebuilds discovery sockets (shared concern with E2). Resolve the saved device by a stable identity, not a drifting index.

---
▸ 2026-09-22T15:38:34Z [Joel Webber]
Basic cpal output WORKS (default device, f32, source-rate match, clean drain) and is proven by canon play-file. NOT yet done (the rest of this yak): device-loss + sleep/wake recovery, DeviceChanged transition wiring, stable device identity (not index), and resampling when the device can't serve the source rate (currently an honest UnsupportedFormat error). Staying shaving until recovery lands.

---
▸ 2026-09-22T17:26:34Z [Joel Webber]
The controllable engine (AudioPlayer: start/pause/resume/stop, volume/mute, EngineEvents, shared clock) now drives real streaming playback over the control plane. Still open on THIS yak: device-loss + sleep/wake recovery, the DeviceChanged transition wiring, stable device identity, and resampling when the device can't serve the source rate.
