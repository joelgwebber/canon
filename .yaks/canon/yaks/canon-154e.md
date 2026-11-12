---
id: canon-154e
title: Local output can't play when the device's channel count differs from the track's
status: hairy
type: bug
priority: 2
created: '2026-09-23T20:41:21Z'
updated: '2026-11-12T19:31:35Z'
parent: canon-b192
tags:
- audio
---

Found in the canon-7f16 cleanup (2026-09-23): `canon play-file` on the mono test assets fails with "unsupported output format: device has no f32 output config for 1 channels". engine::choose_config only accepts configs whose channel count equals the source. Tidal carries mono masters (and local libraries will too), so a mono track cannot play on the Mac local output. Fix: open the device at its own channel count and up/down-mix in the feed (mono -> both channels; more -> a fold-down), next to the resampler. Network outputs are unaffected (the FLAC tap encodes the source channels). Verify with the mono assets in crates/canon-audio/tests/assets and a real Tidal mono track.

---
▸ 2026-11-12T19:31:35Z [delta-paused-drop]

The reverse case, seen 2026-11-12: a **stereo** track on a **mono device**. Joel's Px8 headphones on the Mac were in Bluetooth hands-free mode (HFP: `Output Channels: 1`, `Current SampleRate: 16000` in `system_profiler SPAudioDataType`, the Px8 also the default input). Every track failed in `choose_config` with:

```text
unsupported output format: device has no f32 output config for 2 channels
```

The Mac speakers and Tunes played the same tracks fine.

The fix this yak already names covers it: open the device at its own channel count and mix in the feed. Stereo to mono is the fold-down. Everything else on the Mac plays to a headset in this mode, so canon refusing is the odd one out.

Also worth doing alongside it: log the device's mode when it is this poor (mono, 16 kHz), so a listener hearing call-quality audio learns why.

What happens to the queue when the device can't be opened at all is canon-df3d.
