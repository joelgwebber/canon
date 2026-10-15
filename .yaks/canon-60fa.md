---
id: canon-60fa
title: Tell clients when an output is lost, and why
status: shorn
type: bug
priority: 2
created: '2026-10-04T16:24:10Z'
updated: '2026-10-04T16:50:11Z'
parent: canon-5303
labels:
- sink
verify: cargo test -p canon-core lost && cargo test -p canon-core sink_failure && cargo test -p canon-tui lost_output
---

A lost renderer is invisible to every client: SinkFailed only moves the player's sink to local and restarts there, the snapshot's error field belongs to PlaybackState::Error (and every start clears it), and the reason only reaches the server log. Carry the reason on SinkFailed, keep it in the snapshot as state (sink_lost: which output, and why) until the listener next chooses an output, and show it in the TUI's status line and in canon control.

![lost-output-tui-frame](canon-60fa/lost-output-tui-frame.txt)

---
▸ 2026-10-04T16:49:42Z [Joel Webber]
Headless TUI against a test daemon (:7399, netfault preloaded) on Library display: frame after the Cast link was dropped and every reconnect refused for 30s. Header moves to 'Local output', status line says 'lost Library display: its connection dropped (cast status poll: Broken pipe (os error 32))…'.

---
▸ 2026-10-04T16:49:59Z [Joel Webber]
verify: `cargo test -p canon-core lost && cargo test -p canon-core sink_failure && cargo test -p canon-tui lost_output` -> PASS (exit 0)

---
▸ 2026-10-04T16:50:11Z [Joel Webber]
Shorn 2026-10-04. SinkFailed now carries why; the player keeps it as snapshot.sink_lost {sink, why} until the next SelectSink (state, not an error: playback carried on). Every fail-back site passes a reason. TUI turns a new sink_lost into a notice naming the output (also for a client that connects after); canon control prints 'lost output <id>: <why>' once. On hardware (test daemon :7399, Library display, faults via scripts/netfault.c): canon control printed 'lost output Google-Nest-Hub-a2395…: its connection dropped (cast status poll: peer closed connection without sending TLS close_notify…), and it wasn't back within 30s: sink: cast connect 192.168.0.82:8009: Connection refused (os error 111)'; the TUI frame is attached. Evidence for the failing-local-playback case on a box with no listener is canon-940b's question.
