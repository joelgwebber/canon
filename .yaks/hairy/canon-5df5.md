---
id: canon-5df5
title: Engine network output path (realtime-paced PCM tap + clock)
type: task
priority: 2
created: '2026-09-23T00:24:01Z'
updated: '2026-09-23T00:24:01Z'
parent: canon-dde4
labels:
- audio,sink
---

AudioPlayer::start gains an output target: Local (existing cpal device-session loop) vs Network(Box<dyn PcmSink>). The Network path decodes -> resamples -> submits PCM to the sink, paced to wall-clock realtime so it stays near the broadcaster's live edge (bounded Cast buffer), and advances the shared FrameClock by frames fed (no cpal callback exists on this path). Honors pause/stop. Offline-testable with a mock PcmSink (frames arrive; clock advances; pacing ~ realtime).
