---
id: canon-7c6f
title: Device identity separate from protocol endpoint (one speaker, many protocols)
type: task
priority: 2
created: '2026-09-23T14:50:48Z'
updated: '2026-09-23T14:50:48Z'
parent: canon-7718
labels:
- arch
- sink
---

From canon-ba30 (F). SinkId is the mDNS service name, so a speaker that speaks Cast and DLNA (Tunes) will appear with two unrelated ids once canon-685a lands. Introduce a device identity (UPnP UDN / Cast id / IP as fallback) that groups endpoints, and decide how list_sinks presents it. Proposed: one entry per physical device with a preferred protocol, plus the available protocols listed so an A/B remains possible (for example sink Tunes@dlna). This answers the list_sinks question recorded on canon-685a.
