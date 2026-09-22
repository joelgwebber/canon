---
id: canon-caae
title: 'DSP chain: ReplayGain, EQ, crossfeed, crossfade'
type: task
priority: 2
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T02:01:38Z'
parent: canon-b192
labels:
- audio
- dsp
---

In-callback, in-place, f32, pre-allocated state: ReplayGain (scalar from Tidal EBU-R128 tags, track/album, clip guard) -> EQ (RBJ biquad cascade; crate: biquad) -> crossfeed (Bauer ~700Hz for headphones) -> volume. Equal-power crossfade near track boundaries. All realtime-safe by construction. Network-sink PCM taps (E) branch pre-EQ so the remote gets clean audio at its own volume.
