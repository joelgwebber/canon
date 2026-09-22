---
id: canon-b192
title: Audio pipeline — decode → DSP → local output
type: feature
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T21:32:37Z'
labels:
- audio
---

Encoded input -> decoded PCM -> DSP -> local device. Single bounded PCM ring between a decode task and a realtime-safe output callback. This is the latency-sensitive core that justified going native.
