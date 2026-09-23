---
id: canon-dde4
title: Chromecast control + state feedback
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-23T00:24:18Z'
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
