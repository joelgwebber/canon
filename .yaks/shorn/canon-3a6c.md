---
id: canon-3a6c
title: Stream watchdog fails an idle network output back to local
type: bug
priority: 2
created: '2026-09-23T20:52:50Z'
updated: '2026-09-23T20:57:37Z'
parent: canon-7718
labels:
- sink
- network
verify: cargo build -p canon-daemon && cargo test -p canon-daemon
---

Found on metal while verifying canon-ba9d. The watchdog counts 'no consumers' whenever the newest stream isn't drained, including when nothing is loaded at all: a speaker selected with an empty queue, after stop, or after a track that failed to open. 20s later (10s startup grace + 10s absence) it logs 'renderer stopped consuming our stream' and fails back to local. Absence only means a takeover while a load is in flight (NetworkSession.current is Some); with nothing loaded the renderer has nothing to pull.

---
▸ 2026-09-23T20:57:35Z [Joel Webber]
Watchdog asks loaded(epoch) -> Option<bool> (None = no longer the live session; Some(false) = nothing of ours loaded, NetworkSession.current is None) and only counts absence while loaded. On metal 2026-09-23: sink Tunes, sleep 35 with an empty queue -> still (1, idle, sink LS50-Wire), no warning; enqueue Army of Me plays (5, playing 0->11138); stop -> (6, idle, LS50-Wire) held through sleep 30, serve.log has no "stopped consuming" line (the old build logged it at ~20s idle).

---
▸ 2026-09-23T20:57:37Z [Joel Webber]
verify: `cargo build -p canon-daemon && cargo test -p canon-daemon` -> PASS (exit 0)
