---
id: canon-a70f
title: 'Wire cast sinks into the controller: SelectSink + fail back to local'
type: task
priority: 2
created: '2026-09-23T02:03:10Z'
updated: '2026-09-23T16:34:29Z'
parent: canon-7718
depends_on:
- canon-e920
labels:
- sink
- api
- arch
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

---
▸ 2026-09-23T16:23:54Z [Joel Webber]
REGROWN 2026-09-23: auto-advance on a network sink has never worked. On metal (Tunes, Cast): enqueue 33348478 + 520285418, seek 3:44, sleep 35:
  (6, loading, index 0, Army of Me) pos 223654 -> (7, playing, index 0, Army of Me) pos 223654 -> 253385
The track is 3:54 (234000 ms). Position ran 19s past the end, the queue stayed on index 0, and the device reported player_state Playing / idle_reason None on all 74 polls. Cause: the StreamBroadcaster lives for the whole network session and is never closed at track end, so the HTTP body the renderer pulls never ends. The renderer never reaches EOF, never reports FINISHED, and just starves silently. Any earlier auto-advance evidence would have been on the local path. Fix: canon-e920 (one stream per load, whose body ends when the track has been fed).

---
▸ 2026-09-23T16:34:24Z [Joel Webber]
Fixed: network auto-advance works. It took three things, each found on metal in turn:
1. Per-load streams whose body ends when the track has been fed (canon-e920). Without that the renderer never reaches EOF.
2. Cast LOADs as StreamType::Buffered, not Live. Even with the body ending, a Live stream EOF left the receiver reporting Playing into silence (position ran to 4:22 on a 3:54 track).
3. A receiver that has finished our media drops the media session and answers every later poll with entries: []. The one-off unsolicited IDLE/FINISHED arrives between polls and is lost. cast::media_gone reports that as Ended (edge-filtered once per load). Trace before the fix: s=4 Playing Some(10.65) at stream origin 223.654 = 3:54.3, then 48 polls of Status { entries: [] }.
Also: the stream watchdog no longer counts a fed-in-full stream as a takeover (it fired and failed back to local at the first attempt), and the player clamps position to the track duration (it extrapolated 2.5s past the end before the Ended landed; test position_never_runs_past_the_end_of_the_track).

On-metal evidence, Tunes over Cast, (seq, state, queue index, title) runs:
  enqueue 33348478 + 520285418 / seek 3:44 / sleep 40:
  (7, playing, 0, Army of Me) pos 223654->236553
  (8, loading, 1, Backstabber) -> (10, playing, 1, Backstabber) pos 0->23966     auto-advanced on the speaker, no warnings
  Transport pass (seek +30 / pause / play / next / prev / stop):
  (7 playing 0 39938->46012) (8 paused 46221 held 8s) (9 playing ->50835) (12 playing 1 Backstabber 0->9006) (15 playing 0 Army of Me 0->7090) (16 idle), with no warnings: stop did not auto-advance.

---
▸ 2026-09-23T16:34:29Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)
