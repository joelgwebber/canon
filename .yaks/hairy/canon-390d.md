---
id: canon-390d
title: 'Device path refinements: proactive switch detection + sinc resampling'
type: task
priority: 2
created: '2026-09-22T19:03:19Z'
updated: '2026-09-22T19:03:19Z'
parent: canon-b192
labels:
- audio,sink
---

Follow-ups to canon-940d: (1) proactively detect a DEFAULT-device switch (not just a hard cpal error) — a stall/health watchdog on the frame clock or platform device-change notifications — so an output change with no error still fails over; (2) replace the linear resampler with a sinc-quality one (rubato) for fidelity on rate-mismatched devices. Deferred: reactive reopen-on-error + linear resampling already land in canon-940d.
