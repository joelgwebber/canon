---
id: canon-4a94
title: Daemon lifecycle + task supervision
type: task
priority: 2
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T03:11:16Z'
parent: canon-16b7
labels:
- architecture
verify: cargo build -p canon-daemon
---

Startup/shutdown, a supervised set of long-lived tokio tasks (source session, discovery, API), and clean teardown. Tideway learned that OS framework callbacks (CoreAudio route listener, MediaRemote) have hard thread-affinity and must hop to owned workers — bake that discipline into the task model, not into ad-hoc threads.

---
▸ 2026-09-22T03:11:16Z [Joel Webber]
verify: `cargo build -p canon-daemon` -> PASS (exit 0)

---
▸ 2026-09-22T03:11:16Z [claude]
Daemon supervision skeleton in canon-daemon/src/main.rs: #[tokio::main] inits tracing (env-filter, info default), spawns the PlayerHandle, runs a supervised snapshot-logger task (subscribe -> borrow_and_update -> log, exits when the actor drops), logs 'player state core online', then awaits a cross-platform shutdown_signal (Ctrl-C or unix SIGTERM) and tears down (abort logger, drop handle). Evidence: cargo build -p canon-daemon clean; clippy clean; bounded run logs 'canon daemon starting' + 'player state core online' + the seq-0 idle snapshot and stays resident until killed.
