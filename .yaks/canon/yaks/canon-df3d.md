---
id: canon-df3d
title: Local output with no f32 stereo config skips the whole queue instead of stopping
status: hairy
type: bug
priority: 2
created: '2026-11-12T19:23:19Z'
updated: '2026-11-12T19:23:19Z'
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
