---
id: canon-5afb
title: Authoritative player state machine + event bus
type: task
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T03:09:36Z'
parent: canon-e284
labels:
- state
verify: cargo test -p canon-core
---

enum State { Idle, Loading, Playing, Paused, Ended, Error } owned by a single control actor/task; only it mutates transport state, under one lock, bumping a monotonic seq. All commands (play/pause/seek/load) and all inputs (decode EOF, device lost, sink failed, external-controller takeover) are messages to this actor. Subscribers get events off a broadcast bus.

---
▸ 2026-09-22T03:09:35Z [Joel Webber]
verify: `cargo test -p canon-core` -> PASS (exit 0)

---
▸ 2026-09-22T03:09:36Z [claude]
Player state actor landed in canon-core/src/player.rs. PlayerHandle{spawn,command,engine,subscribe,snapshot,clock} fronts a single Actor task. Commands (Load/Play/Pause/Stop/Seek/SetVolume/SetMuted/SelectSink) arrive from the API/MCP side; EngineEvent (Loaded/Ended/Failed/DeviceChanged/SinkFailed) arrives from the audio/sink side. Snapshots published via tokio watch (snapshot-then-deltas, reconcile by seq); seq bumps per transition; a 250ms tick refreshes derived position while playing (position/rate let clients interpolate). Evidence: cargo test -p canon-core = 4 passed; clippy clean.
