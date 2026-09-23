---
id: canon-a70f
title: 'Wire cast sinks into the controller: SelectSink + fail back to local'
type: task
priority: 2
created: '2026-09-23T02:03:10Z'
updated: '2026-09-23T02:35:08Z'
parent: canon-7718
labels:
- sink,api
verify: cargo test --workspace
---

canon-dde4 proved the Cast path via the 'canon cast' harness; now route it through the daemon proper. Command::SelectSink(<discovered id>) should: build a CastSink, start the LAN stream server on the chosen iface, restart the current track on Output::Network at the current position (single-active-output, per the dde4 decision), and feed CastEvents INTO PlaybackController's state actor (Playing/Paused/Ended/Superseded/Failed -> player transitions, incl. auto-advance on Ended). Watch SinkHealth and fail back to Output::Local on Failed (EngineEvent::SinkFailed already exists). Also expose the discovery snapshot over the WS API so clients can list/select sinks.

---
▸ 2026-09-23T02:34:53Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-09-23T02:35:08Z [Joel Webber]
Shorn. Cast sinks are now driven by the daemon over the WebSocket, not just the harness.
- canon-core: added SinkInfo (serialisable sink descriptor, reserved 'local' id + is_local) and ControlPlane::sinks() with a local-only default, so canon-api can publish a device list WITHOUT depending on canon-sink (dependency direction preserved).
- canon-api: new 'list_sinks' op -> ReplyData::Sinks.
- controller: Inner gained cast: Option<CastSession> (sink + broadcaster + url + server task; Drop aborts the server and drops the CastSink, which stops the receiver — teardown has no separate step to forget). select_sink() switches output and RESTARTS the current track at the current position (single-active-output per the dde4 decision); each track installs a FRESH FlacTap (new STREAMINFO) and re-issues LOAD. play_index_at routes to Output::Network when a cast session is live, else Output::Local. Discovery is Option<Arc<DiscoveryService>> so local playback still works when discovery can't start (macOS permission), reporting a clean Unsupported error instead of failing obscurely.
- Device reports fold INTO the state machine (run_cast_events): Playing/Paused -> player commands, Ended -> the SAME on_engine_event path as a local end-of-track so the queue auto-advances identically on either output, Superseded/Failed + SinkHealth::Failed -> fail_back_to_local (drops the session only if still current, emits EngineEvent::SinkFailed, resumes locally from the last position).
Evidence:
- cargo test --workspace green (clippy 0 warnings, fmt clean); new ws test asserts list_sinks returns the local entry first with the reserved id.
- LIVE end-to-end over the ws against the KEF 'Tunes': list_sinks showed 5 sinks; played LOCAL (pos 8138) -> select cast ok -> CASTING on the device (pos 21918) -> select local ok -> BACK on local at pos 22801, i.e. it RESUMED rather than restarting from zero.
Still open: canon-bc84 (position leads real cast audio by the receiver buffer), and external takeover is unit-tested but not yet observed live.
