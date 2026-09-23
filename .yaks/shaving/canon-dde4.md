---
id: canon-dde4
title: Chromecast control + state feedback
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-23T01:48:10Z'
parent: canon-7718
labels:
- sink
- network
---

Connect + play_media(url, audio/flac, LIVE) + track-change re-issue, via the Cast app framework (crate: rust_cast). CLOSE THE FEEDBACK LOOP tideway left open: route MediaStatus (player_state, idle_reason, external takeover) back INTO the state machine (B) as an input, so a phone hijacking the session surfaces instead of the app asserting stale control.

---
▸ 2026-09-23T00:16:17Z [Joel Webber]
Dep survey (deps added to canon-sink Cargo.toml but NOT yet committed, pending TLS decision):
- flacenc 0.5.1 (pure Rust): STREAMING FEASIBLE. Use coding::encode_fixed_size_frame(config, framebuf, frame_number, &stream_info) per PCM block; StreamInfo::set_total_samples(0) for unknown live length; BitRepr+ByteSink serialize a standalone fLaC+STREAMINFO header and each Frame to bytes. Maps onto 21f7 set_header (once) + push (per frame). No blocker.
- rust_cast 0.21.0 (pure Rust CASTv2): builds on 1.95 BUT hard-wires rustls default provider = aws-lc-rs -> aws-lc-sys (bundled C, built via cmake+cc). Additive feature unification means it can't be stripped transitively; avoiding it needs a rust_cast patch (rustls default-features=false + ring or a pure-Rust provider). Runtime is still a self-contained static binary; the cost is a C toolchain (cmake) at BUILD time + a big C dep. Conflicts with the pure-Rust preference. OPEN QUESTION to user: accept aws-lc-sys for now (recommended: it works, self-contained at runtime; file a follow-up to revisit the provider) vs. hold Cast until we can get rust_cast onto ring/pure-Rust.

---
▸ 2026-09-23T00:24:18Z [Joel Webber]
DESIGN DECISION for v1 Cast (recorded; evolves how cf48's primitive is used):
SINGLE ACTIVE OUTPUT, switch-restarts-playback. Selecting a sink stops the current output and (re)starts the current track on the new output at the current position (like a seek). Rationale:
- The headless-server case (NO local audio device, cast-only) is a first-class user requirement, so the architecture must play to a network sink with no local device open. That rules out the 'keep local open but muted' dual-output model.
- With single output, local silence = local simply not selected; there is no separate mute flag to leak. So the cpal callback does NOT consult LocalGate in v1. The cf48 OutputRoute/LocalGate/RouteGuard primitive is correct and retained for SIMULTANEOUS local+network (multi-room) later; it is not wired into the hot path now (consistent with cf48's deferral note). Filing multi-room as the home for it.
Build pieces: canon-dfdd (PCM->FLAC encoder tap, PcmSink), canon-5df5 (engine Local|Network output target), then CastSink (connect/LOAD/MEDIA_STATUS->state actor) + controller sink-select/restart.
KNOWN v1 LIMITATION -> refinement yak: position is driven by frames-fed (leads actual Cast audio by the Cast buffer, ~1-3s). Reconcile to MEDIA_STATUS-reported position later.
Also: switching output re-resolves the Tidal stream for now (brief gap); keeping the decoder and swapping only the sink is a later optimization.

---
▸ 2026-09-23T01:33:59Z [Joel Webber]
rust_cast API constraints (read from vendored 0.21 source — these dictate the CastSink architecture):
- BLOCKING/sync client (TcpStream + rustls), NOT async. CastDevice::receive() blocks.
- MessageManager holds ONE mutex over the stream, and read() keeps it for the entire blocking read. So a second thread calling send() while another blocks in receive() DEADLOCKS. => Architecture: exactly ONE owning thread does all Cast I/O (sends + receives), driven by a command channel; it sets a TCP read timeout so it can interleave commands, heartbeat pongs, and status reads. Never share CastDevice across threads.
- Must answer Heartbeat Ping with pong() or the device drops the connection.
- Flow: connect(host,8009) [or connect_without_host_verification] -> receiver.launch_app(&CastDeviceApp::DefaultMediaReceiver) -> connection.connect(transport_id) -> media.load(transport_id, session_id, &Media{content_id: our stream URL, stream_type: StreamType::Live, content_type: 'audio/flac', ..}) -> returns Status with media_session_id; then media.pause/play/stop/seek(transport_id, media_session_id).
- Status feedback: ChannelMessage::Media(MediaResponse::Status(Status{entries:[StatusEntry{player_state, idle_reason, current_time, media_session_id, ..}]})). PlayerState{Idle,Playing,Buffering,Paused}; IdleReason{Cancelled,Interrupted,Finished,Error}. External takeover shows up as a status whose media_session_id differs from ours (or idle_reason Interrupted/Cancelled).

---
▸ 2026-09-23T01:48:10Z [Joel Webber]
ON-METAL ATTEMPT 1 (KEF 'Tunes' 192.168.0.205) — everything works up to the receiver fetching our stream; blocked by a macOS inbound-LAN restriction on our own listener.
WHAT WORKED: discovery by name -> Tunes; stream server bound en0 (192.168.0.67:PORT); Tidal resolve (Flac 44100x2) -> FlacTap -> engine Output::Network fed frames ('[engine] feeding at 44100 Hz'); Cast connect + launch DefaultMediaReceiver + LOAD ACCEPTED, reply: media_session_id=5/6, content_id=our URL, content_type=audio/flac, stream_type=Live, extended_status.player_state=Loading, supported_media_commands=274447. So the whole control path + classify plumbing is correct.
WHAT FAILED: the receiver never connected to the stream server (no consumer-connected log), stayed in Loading, then all subsequent get_status polls returned entries: [] (35 polls).
ROOT CAUSE ISOLATED (new test tests/stream_server_lan.rs reproduces deterministically): fetching our OWN stream server bound on en0 from this machine fails with ConnectionReset (errno 54), while the SAME server on 127.0.0.1 serves correctly (tests/stream_server_live.rs passes) AND a plain Python listener bound on 192.168.0.67 accepts connections fine. So it is not the server code and not the LAN: macOS is resetting INBOUND TCP to OUR process's LAN listener. Local Network permission covers inbound LAN connections too, and cargo test/run binaries are ad-hoc/linker-signed (identity changes every rebuild), so a TCC grant cannot stick to them the way it did for mdns (which went through the already-granted host process).
NEXT: user to run the repro from a context holding its own grant (Terminal.app) — 'cargo test -p canon-sink --test stream_server_lan -- --ignored --nocapture'. If it passes there, re-run 'canon cast' from that same context and the receiver should fetch. Longer-term this is the same family as canon-bb20 (ship a signed .app with the multicast entitlement / stable identity); headless Linux is unaffected.
