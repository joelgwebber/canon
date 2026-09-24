---
id: canon-07b6
title: 'Structured device quirks: warn about known-flaky device/protocol/mode combos'
type: task
priority: 2
created: '2026-09-24T18:22:29Z'
updated: '2026-09-24T18:22:29Z'
labels:
- sink
- network
---

docs/device-quirks.md is the hand-kept record of what real renderers taught us (seeded from canon-c200). Make the actionable part of it structured, so canon can say so when a user picks a combination known to fail, rather than rediscovering it on metal.

Motivating case (canon-c200): KEF LS50 Wireless II over Cast drops our live chunked stream after 2-14 min; the same speaker over DLNA, and a Nest Hub over Cast, are fine.

Shape to settle:
- A quirk = a device matcher + protocol (+ optional output mode) + severity + a one-line consequence + a pointer into docs/device-quirks.md.
- Match on what discovery already knows or can cheaply learn: Cast TXT 'md' (model) / manufacturer, UPnP modelName/manufacturer from the description XML. Not the mDNS instance name (user-renamable) and not the HTTP user agent (only seen after a load).
- Surface it: a warning on the sink in 'sinks' / SinkInfo, a WARN when selected, and possibly bias the preferred protocol for a multi-protocol output (Tunes: prefer DLNA).
- Quirks live in code as data (a table), with docs/device-quirks.md as the prose + evidence. Decide whether the doc or the table is canonical and keep them from drifting.
