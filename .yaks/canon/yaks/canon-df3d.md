---
id: canon-df3d
title: Local output with no f32 stereo config skips the whole queue instead of stopping
status: hairy
type: bug
priority: 2
created: '2026-11-12T19:23:19Z'
updated: '2026-11-12T19:31:36Z'
tags:
- audio
---

Found 2026-11-12 on the j15r server's log, from the Mac player (player id `mac`). From 19:18:35Z every track the player was asked to play failed after about 10 s and was skipped, until the queue ran dry (`state=Idle` 19:20:09Z):

`WARN canon_core::player: skipping "Contrapasso": unsupported output format: device has no f32 output config for 2 channels`

The text comes from `choose_config` in `crates/canon-audio/src/engine.rs`: the output device cpal chose offers no f32 configuration with exactly 2 channels (a mono Bluetooth headset in call mode, a display or HDMI device that only lists 8 channels, an aggregate device, and similar). The server is not involved: its `/audio` endpoint served each track fine (`canon::audio: streaming ...` lines), the failure is on the executor's local output.

Two problems:

- `choose_config` has no fallback: it accepts only f32 with the stream's own channel count, so a device that offers f32 mono, i16, or more than 2 channels is "unsupported" instead of being used (down/up-mix, convert).
- A failure of the **device** skips the **track**, and so burns the whole queue in a few seconds, the same way the Tidal refusal did (canon-9770). A device that cannot be opened should stop the player with the reason (and let the client pick another output), not advance.

Also unknown: which device the Mac player had. Its log (`~/Library/Application Support/canon/logs/canon.log`) names it at session start if `canon_audio=debug` was on.

---
▸ 2026-11-12T19:31:36Z [delta-paused-drop]

The device, answered (2026-11-12): Joel's **Px8** Bluetooth headphones, in hands-free (HFP) mode. `system_profiler SPAudioDataType` showed them as default output and input with `Output Channels: 1` and `Current SampleRate: 16000`. Away from them (Mac speakers, Tunes) the same queue played.

It happened twice: 19:18–19:20Z, and again from 19:26Z after `canon player` was restarted at 19:24:59Z. It skipped "Contrapasso", "Your Ghost Again", "Scars", "Part III", "The Frayed Ends of Sanity" and on.

The first bullet (a mono device) is canon-154e's fix, which now covers mono devices as well as mono tracks. This yak is the second bullet: a failure of the output must not count against tracks.

- The rule: a `PlayError` about the device (`NoDevice`, `UnsupportedFormat`, a device that won't open) is an output failure. It should reach the player as one, like `SinkFailed`, not as `EngineEvent::Failed` for the track.
- Per Joel's answer on canon-940b, a lost output pauses at the listener's place rather than playing on. The same fits here: stop or pause with the reason in `sink_lost`/`error`, so the client can pick another output and play resumes there.
