---
id: canon-583a
title: 'Protocol-neutral sink seam: impl Sink for Cast, controller holds dyn Sink'
type: task
priority: 1
created: '2026-09-23T14:50:19Z'
updated: '2026-09-23T16:03:30Z'
parent: canon-7718
labels:
- arch
- sink
verify: cargo test --workspace
---

From canon-ba30 (A)+(C). The Sink trait has no impls: CastSink shadows it with inherent methods, and the controller is hardwired to CastSession/CastSink/CastEvent. Sink::start(&TrackMeta) also does not match what a renderer does. Reshape Sink around what Cast actually needs: load(url, meta), play/pause/stop, set_volume, maybe load_next(url), a device-event receiver, and health. Impl it for CastSink, and have the controller hold a protocol-neutral NetworkSession { Box<dyn Sink>, broadcaster, server }. Hoist a neutral RendererEvent {State, Position, Ended, Superseded, Failed} and the edge-dedup (CastEvent::is_edge + last) into canon-sink so each protocol only classifies. Collapse the duplicate failure signal (SinkHealth::Failed and CastEvent::Failed both fire). Success test: canon-685a adds a module and a match arm, not controller surgery. Cast must still pass the AGENTS.md on-metal transport run afterwards.

---
▸ 2026-09-23T15:50:55Z [Joel Webber]
Design:
- canon-core::sink: Sink becomes a sync, &self, object-safe command surface: id, kind, load(url, meta), play, pause, stop, set_volume, set_muted. Implementations queue to their own I/O task, and a failure surfaces as an event. No async and no &mut, so the controller can call it under its lock without awaiting. Seek is left off: seeking a renderer is a re-LOAD of a stream that starts at the seek point. load_next waits for canon-e920.
- RendererEvent {State(RendererState), Position, Ended, Superseded, Failed} moves to core next to the trait (RendererState moves from player.rs). is_edge() lives on it.
- SinkHealth is deleted. It duplicated CastEvent::Failed. The event stream is the one liveness signal, and its closing is itself a signal.
- canon-sink: EdgeFilter (edge dedup, the exact semantics of the old report() last cache), plus canon_sink::connect(&DiscoveredDevice) -> (Box<dyn Sink>, events), the one protocol match. DLNA adds a module and an arm there.
- Controller: NetworkSession {sink: Box<dyn Sink>, broadcaster, url, server, epoch}. Event and watchdog tasks key on the session epoch, not SinkId, so re-selecting the same speaker cannot have a stale task tear down the new session. A closed event channel means fail back, a no-op when epoch-stale.

---
▸ 2026-09-23T16:03:22Z [Joel Webber]
Done. Sink is now a sync, object-safe command surface (load/play/pause/stop/set_volume/set_muted), and CastSink implements it. RendererEvent and RendererState live in canon-core::sink. SinkHealth is gone, so a failure is reported once (Failed), and a closed event stream means the session ended. canon_sink::connect(&DiscoveredDevice) is the only protocol match, and EdgeFilter is the only level-vs-edge rule. The controller holds NetworkSession { Box<dyn Sink>, epoch, … } and never names Cast. Its event and watchdog tasks key on the epoch and ignore reports from a session that is no longer current. LOCAL_SINK is deleted in favour of SinkInfo::local(). canon-583a success test met: DLNA is now a module plus the Dlna arm in renderer::connect.

Found and fixed while verifying (both pre-existing, both reproduced at HEAD 50111e6 first):
1. Re-selecting the speaker you are already on wedged playback in Loading forever. Cause: Cast teardown called receiver.stop_app, but the Default Media Receiver is shared, so the new session is handed the same app instance, and the old session teardown (landing after the new LOAD) killed it. At HEAD: selecting Tunes -> loading 0:06 -> loading 0:03 ... (15s, never plays). Fix: teardown stops our media session and disconnects, and leaves the app to time out.
2. A new session first poll (before its LOAD is accepted) classified the previous media on the shared app as ours: a spurious playing 0:07 before loading 0:03, and a foreign position fed to reconcile. classify and reported_position now report nothing until our media session exists (unit test: nothing_is_ours_before_our_load_is_accepted).

Observed, not changed: the Kitchen (Nest Hub) first reports after a switch run about 5s ahead of the segment-aligned restart (playing 0:55 -> 1:00) and then briefly rebuffer. Each switch/seek also restarts up to one segment early, by design (segment-granular open_stream_at). Worth watching when DLNA lands.

---
▸ 2026-09-23T16:03:27Z [Joel Webber]
On-metal evidence (Tunes = KEF LS50 Wireless II, Kitchen = Nest Hub), canon control: sink Tunes / enqueue 33348478 / sleep 20 / seek +30 / sleep 10 / sink Tunes / sleep 12 / sink Kitchen / sleep 12 / sink local / sleep 5 / stop:
  selecting Tunes -> loading 0:00 -> playing 0:00 ... 0:15
  seek +30        -> loading 0:17 -> loading 0:43 -> playing 0:43 ...
  selecting Tunes -> loading 0:52 -> loading 0:51 -> playing 0:51, 0:53, 0:54, 0:55, 0:56   (re-select: plays; wedged at HEAD)
  selecting Kitchen -> loading 1:00 -> loading 0:55 -> playing 0:55 -> 1:00 -> loading 1:00 -> playing 1:00, 1:01
  selecting Local output -> loading 1:02 -> playing 0:59 ... 1:04
  stop -> idle 0:00
serve.log at RUST_LOG=warn: empty (no spurious fail-back or session-lost warnings across three switches).
Green bar: cargo fmt --check clean, clippy --workspace --all-targets zero warnings, cargo test --workspace all pass (canon-sink lib 38, canon-core 28), cargo build --workspace ok.
Pause/volume still do not reach the device, as expected. That is canon-44f4, next.

---
▸ 2026-09-23T16:03:29Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)
