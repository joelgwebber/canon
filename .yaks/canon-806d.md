---
id: canon-806d
title: 'Executor role: effects down, engine events up, over the WS'
status: shaving
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-18T04:18:48Z'
parent: canon-88a7
depends_on:
- canon-e329
- canon-973d
source: arch
needs: human
verify: cargo test -p canon-core executor && cargo test -p canon-core station && cargo test -p canon-core losing_the_executor && cargo test -p canon-api --test control_plane a_remote_executor
---

Today `Effect` and `EngineEvent` travel over an in-process mpsc between the actor and `PlaybackController`. Make them serialisable protocol messages and add an executor role to the WebSocket: an executor connection receives its player's effects and sends back generation-tagged engine events, renderer reports, and its discovered outputs (`SinkInfo`). The actor keeps judging staleness by generation exactly as now; the network adds latency, not new semantics.

Also: an executor disconnecting puts its actor into a disconnected/paused state (no fail-back to local; there is no local on the server).

Parent decisions: canon-88a7.

---
▸ 2026-10-17T20:48:43Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T21:37:00Z [delta:The Great Decoupling]

verify: `cargo test -p canon-core executor && cargo test -p canon-core station && cargo test -p canon-core losing_the_executor && cargo test -p canon-api --test control_plane a_remote_executor` -> PASS (exit 0)

---
▸ 2026-10-17T21:37:00Z [delta:The Great Decoupling]

Done in code; one hardware check outstanding (see ask).

What changed:
- `Effect`, `EngineEvent` (and `RendererState`) are serde types, times as ms. New wire types in canon-core/src/executor.rs: `ExecutorOrder` (effect + signed audio path for starts/prepares; `select` an output) and `ExecutorReport` (engine event + generation; outputs; select answer).
- canon-core/src/station.rs: `Station` = a player's actor + its one executor. It forwards effects to the current executor; a start with none fails at once (`ExecutorLost`), so nothing sits in Loading waiting for sound. `SelectSink` asks the executor to connect first and only then commands the actor, as the old controller did. A newer executor supersedes the last (its orders end, its reports are dropped); the current one leaving → `ExecutorLost`: halt, keep the place, error says why, next play resumes there.
- The daemon controller is now a pure executor: no PlayerHandle; it learns volume/mute/paused/selected output from orders, and reports outputs (local + discovery, re-sent on change). Every player gets the server's own controller as its first executor, so `canon serve` behaves as before.
- WS: `become_executor` (ack) then `executor` orders down / `executor` reports up on the same connection, so a player with a UI is one connection with both roles.
- Renderer reconnect now reports `OutputRestored` instead of re-selecting through the actor itself.

Evidence:
- Green bar: fmt clean, clippy 0 warnings, test --workspace ok, build ok.
- Tests: wire round trip; Station: executor gets volume then effects with the audio path, its reports drive the actor, its outputs are listed; select waits for the executor and a refusal moves nothing; no executor → start fails at once keeping the queue, select refused; supersede + leave semantics. Actor: `losing_the_executor_holds_the_listeners_place` (halts, keeps queue, resumes at ~7s), untagged non-output reports dropped. API: `a_remote_executor_carries_out_its_players_playback` over a real socket: start with no executor fails with "no player is connected"; `become_executor`; executor's outputs show in `list_sinks`; `play` → executor gets `start` with `/audio/…`; its `loaded` report makes every controller see `playing`; dropping it → idle + "disconnected".
- Live (own --state-dir, library copy, no credentials): serve on 7395/7351; `canon control sinks` lists local + Basement, Kitchen, Library display, Living Room, Tunes via the controller's reported outputs (18s discovery); `add 33348478` → error state because tidal can't stream without a login, which is expected here.

Decision: one `Station` per player in canon-core owns the actor↔executor seam, rather than the daemon controller knowing about remote executors. The in-process controller and a remote player are then the same kind of thing to the actor.
Decision: the server's controller is attached by default (behaviour unchanged, no-local-device constraint intact via renderers). A remote executor supersedes it; when the remote leaves, the player gets no executor back (no surprise sound from the server box). Rejected: falling back to the server's controller automatically.
Decision: output selection is a request/answer through the executor (`select`/`selected`) because only the executor can reach the speaker; the actor changes `sink` only after success, as before.
Decision: both roles on one connection, rather than a separate `/executor` socket: the agreed player+UI model is one process, and auth/player identity are already per connection.

---
▸ 2026-10-17T21:37:00Z [delta:The Great Decoupling]

asked: needs human

Hardware check needed once these commits are on Joel's checkout (needs the real state dir and Tidal login): canon serve on its own port, then the AGENTS.md session: sinks; sink Tunes; enqueue 33348478; sleep 25; seek +30; pause; play; and sink Tunes@dlna then back to local. Expect it to behave exactly as before this change: the server's controller is now an executor behind a Station, and selection/reconnect paths moved. Also pull Tunes' network mid-track (netfault refuse-8009) to check OutputRestored resumes it.

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

Live, Joel's state dir (test daemon 7399/7347):
- Server's own controller as executor: local-output session on Tidal passed (see canon-5ce4's note: play, seek, pause/resume, gapless).
- Tunes: `sink Tunes` → `cast load: Failed to load media. It never fetched a stream from http://192.168.0.27:7347/1`; `Tunes@dlna` → not reachable over dlna. Cause: ufw allows only 7346 from the LAN (`### tuple ### allow tcp 7346 … canon`), and the test daemon must use 7347 because Joel's daemon holds 7346. Not a regression signal either way.

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

Still needed (Joel): either `sudo ufw allow from 192.168.0.0/24 to any port 7347` so a test daemon can reach speakers, or run the Tunes session on the real daemon (port 7346) once it's restarted on this build. Script: target/hw-806d.sh.
