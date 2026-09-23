---
id: canon-154e
title: Local output can't play mono (or any channel count the device lacks)
type: bug
priority: 2
created: '2026-09-23T20:41:21Z'
updated: '2026-09-23T20:41:21Z'
parent: canon-b192
labels:
- audio
---

Found in the canon-7f16 cleanup (2026-09-23): `canon play-file` on the mono test assets fails with "unsupported output format: device has no f32 output config for 1 channels". engine::choose_config only accepts configs whose channel count equals the source. Tidal carries mono masters (and local libraries will too), so a mono track cannot play on the Mac local output. Fix: open the device at its own channel count and up/down-mix in the feed (mono -> both channels; more -> a fold-down), next to the resampler. Network outputs are unaffected (the FLAC tap encodes the source channels). Verify with the mono assets in crates/canon-audio/tests/assets and a real Tidal mono track.
