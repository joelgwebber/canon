---
id: canon-7718
title: Sink abstraction & network renderers
type: feature
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T02:01:38Z'
parent: canon-dd79
labels:
- sink
---

Local and network outputs behind one Sink trait, with resilient discovery and clean session lifecycle. This is where tideway was flakiest (the whole tide-4000.x and tide-6fd0 families), so get the shapes right from the start; specific protocol fiddliness (OpenHome, Tidal Connect, gapless) can come later.
