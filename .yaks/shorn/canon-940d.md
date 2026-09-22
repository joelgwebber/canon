---
id: canon-940d
title: cpal local sink + device-loss/sleep-wake recovery
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T19:03:29Z'
parent: canon-b192
labels:
- audio
- sink
verify: cargo test -p canon-audio
---

cpal for enumeration, default-device-change callbacks, exclusive/shared, and the disconnect error path — which is where the DeviceChanged transition (B3) fires. Recovery re-enumerates and reopens (user device, else system default) while the decoder keeps filling the ring for a seamless gap. Register a sleep/wake hook that also rebuilds discovery sockets (shared concern with E2). Resolve the saved device by a stable identity, not a drifting index.

---
▸ 2026-09-22T15:38:34Z [Joel Webber]
Basic cpal output WORKS (default device, f32, source-rate match, clean drain) and is proven by canon play-file. NOT yet done (the rest of this yak): device-loss + sleep/wake recovery, DeviceChanged transition wiring, stable device identity (not index), and resampling when the device can't serve the source rate (currently an honest UnsupportedFormat error). Staying shaving until recovery lands.

---
▸ 2026-09-22T17:26:34Z [Joel Webber]
The controllable engine (AudioPlayer: start/pause/resume/stop, volume/mute, EngineEvents, shared clock) now drives real streaming playback over the control plane. Still open on THIS yak: device-loss + sleep/wake recovery, the DeviceChanged transition wiring, stable device identity, and resampling when the device can't serve the source rate.

---
▸ 2026-09-22T19:03:19Z [Joel Webber]
verify: `cargo test -p canon-audio` -> PASS (exit 0)

---
▸ 2026-09-22T19:03:29Z [Joel Webber]
Shorn: the cpal local sink is now recovery-capable. Output runs as device sessions; a cpal error flips device_failed and the engine reopens (prefer same device by name, else default) with backoff (~10s grace), emitting DeviceChanged for continuous position, decoder keeping its place. Resampling (streaming linear) engages when the device can't serve the source rate instead of hard-erroring. Stable device identity by name, not index. VERIFIED: forced-reopen integration test (DeviceChanged + clock keeps advancing across the reopen) and resampler unit tests; normal Tidal streaming plays through the reworked engine. CAVEAT: real sleep/wake + physical unplug rely on cpal's error callback firing and can only be confirmed on-metal (needs Joel to sleep/unplug mid-playback). Refinements (proactive default-switch detection, sinc resampling) -> canon-390d.
