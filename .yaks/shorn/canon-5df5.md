---
id: canon-5df5
title: Engine network output path (realtime-paced PCM tap + clock)
type: task
priority: 2
created: '2026-09-23T00:24:01Z'
updated: '2026-09-23T00:42:44Z'
parent: canon-dde4
labels:
- audio,sink
verify: cargo test -p canon-audio
---

AudioPlayer::start gains an output target: Local (existing cpal device-session loop) vs Network(Box<dyn PcmSink>). The Network path decodes -> resamples -> submits PCM to the sink, paced to wall-clock realtime so it stays near the broadcaster's live edge (bounded Cast buffer), and advances the shared FrameClock by frames fed (no cpal callback exists on this path). Honors pause/stop. Offline-testable with a mock PcmSink (frames arrive; clock advances; pacing ~ realtime).

---
▸ 2026-09-23T00:42:44Z [Joel Webber]
verify: `cargo test -p canon-audio` -> PASS (exit 0)

---
▸ 2026-09-23T00:42:44Z [Joel Webber]
Shorn. AudioPlayer::start gained an Output param: Local (existing cpal device-session loop, unchanged) vs Network(Box<dyn PcmSink>).
run_network: decode -> submit PCM to the sink -> advance the shared FrameClock by frames fed (no cpal callback on this path), paced to wall-clock realtime kept ~2s (NETWORK_LEAD) ahead so a renderer buffer fills at start without racing the consumer. Honors pause (stops feeding + excludes paused time from pacing) and stop (prompt via pace_sleep). Full-scale PCM (no gain/mute: a network renderer owns its own volume). Decode-exhaustion returns WITHOUT Ended (the renderer is still draining its buffer; Ended comes from the sink's MEDIA_STATUS in the Cast yak); dropping the sink flushes its trailing frame (FlacTap gained a Drop-flush). Switching output = restart on the new output (single active output, v1).
Updated all AudioPlayer::start callers (controller, main, recovery test) to Output::Local.
Evidence: cargo test -p canon-audio incl. tests/network_output.rs (decodes flac_frag.mp4 through the network path into a mock PcmSink; asserts PCM fed + clock advanced); workspace clippy 0 warnings, fmt clean, all suites green.
