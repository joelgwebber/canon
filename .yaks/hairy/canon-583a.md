---
id: canon-583a
title: 'Protocol-neutral sink seam: impl Sink for Cast, controller holds dyn Sink'
type: task
priority: 1
created: '2026-09-23T14:50:19Z'
updated: '2026-09-23T14:50:19Z'
parent: canon-7718
labels:
- arch
- sink
---

From canon-ba30 (A)+(C). The Sink trait has no impls: CastSink shadows it with inherent methods, and the controller is hardwired to CastSession/CastSink/CastEvent. Sink::start(&TrackMeta) also does not match what a renderer does. Reshape Sink around what Cast actually needs: load(url, meta), play/pause/stop, set_volume, maybe load_next(url), a device-event receiver, and health. Impl it for CastSink, and have the controller hold a protocol-neutral NetworkSession { Box<dyn Sink>, broadcaster, server }. Hoist a neutral RendererEvent {State, Position, Ended, Superseded, Failed} and the edge-dedup (CastEvent::is_edge + last) into canon-sink so each protocol only classifies. Collapse the duplicate failure signal (SinkHealth::Failed and CastEvent::Failed both fire). Success test: canon-685a adds a module and a match arm, not controller surgery. Cast must still pass the AGENTS.md on-metal transport run afterwards.
