---
id: canon-a7d6
title: 'Playback controller: source -> engine -> player over the control plane'
type: task
priority: 1
created: '2026-09-22T16:40:33Z'
updated: '2026-09-22T17:26:34Z'
parent: canon-b192
labels:
- audio
- api
verify: cargo test --workspace
---

Tie the pieces into a driveable whole: a controller that on Load resolves a track via a Source, starts the controllable audio engine on the shared FrameClock, and feeds EngineEvents (Loaded/Ended/Failed) back to the player actor; pause/resume/stop/volume flow to both the engine and player state. Exposed to canon-api via a ControlPlane seam so a ws client (or agent) drives real playback and sees state+position in snapshots.

---
▸ 2026-09-22T17:26:22Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-09-22T17:26:33Z [Joel Webber]
Shorn: PlaybackController (impl ControlPlane) ties Tidal source -> streaming engine -> player on the shared FrameClock, forwarding EngineEvents back to the player; pause/resume/stop/volume fan out to engine + state. Verified live over WebSocket: play_track -> loading -> playing with position advancing off the realtime callback; pause holds position; resume advances; stop -> idle. Seek returns a clean Unsupported for now.
