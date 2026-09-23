---
id: canon-bbc7
title: Replace rust_cast with our own async Cast client (push status, no cmake)
type: task
priority: 3
created: '2026-09-23T17:31:17Z'
updated: '2026-09-23T17:31:17Z'
parent: canon-7718
labels:
- arch
- sink
- network
---

Recommendation: replace, not fix upstream.

WHY RUST_CAST IS THE WRONG SHAPE FOR US (rust_cast 0.21, ~5.7k lines):
- Synchronous, with one mutex over the TLS stream held across a blocking read. We cannot listen and command at once, so canon-sink::cast runs one owning thread that alternates draining commands with a 500ms get_status poll. Consequences seen on metal: we lose the unsolicited IDLE/FINISHED broadcast (it lands between polls), and had to infer end-of-track from our media session vanishing (cast::media_gone). Command latency is up to a poll interval, and there is a dedicated OS thread per session.
- Build weight: rustls with its default aws-lc-rs provider means aws-lc-sys, which needs cmake and a C toolchain (canon-d419), plus protobuf pinned to =3.7.2 for a generated CastMessage.
- Upstream: the TLS provider is a small, acceptable PR (a feature to pick ring), which fixes cmake alone. Going async is a rewrite of the crate core API, not a PR anyone would merge.

WHAT WE ACTUALLY USE: TLS to a pinned LAN address with no host verification; the CastMessage envelope (a 4-byte big-endian length, then a protobuf of 7 fields: protocol_version, source_id, destination_id, namespace, payload_type, payload_utf8, payload_binary); and JSON payloads on four namespaces:
- tp.connection: CONNECT/CLOSE
- tp.heartbeat: PING/PONG
- receiver: LAUNCH, STOP, GET_STATUS, SET_VOLUME (level/muted)
- media: LOAD (and QUEUE_LOAD for gapless, see the gapless design yak), PLAY, PAUSE, STOP, GET_STATUS, plus unsolicited MEDIA_STATUS.

PLAN: an async client in canon-sink (or a small canon-cast crate if it wants its own tests/fixtures): tokio + tokio-rustls on the ring provider (or rustls-rustcrypto), with CastMessage hand-encoded (or prost), and serde JSON messages. One tokio task per session selects over commands, inbound frames and a heartbeat timer, with request ids matched to replies. Unsolicited MEDIA_STATUS is classified by the existing pure classify/reported_position/media_gone. Estimate: 600-900 lines with tests (frame codec + message fixtures testable without a device). Keep a slower get_status poll only as a liveness backstop.

WINS: push status (instant edges, FINISHED actually seen), no blocking thread, drop aws-lc-sys/cmake and the protobuf pin (resolves canon-d419), and Cast and DLNA sessions share one task shape. Verify on metal with the full AGENTS.md transport pass, plus auto-advance, re-select of the same speaker, and a Cast<->DLNA handover.
