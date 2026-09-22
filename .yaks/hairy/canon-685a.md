---
id: canon-685a
title: DLNA/UPnP control + GENA state feedback
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T02:01:39Z'
parent: canon-7718
labels:
- sink
- network
---

AVTransport SOAP (SetAVTransportURI/Play/Pause/Seek/GetTransportInfo). Unlike tideway, actually consume device state: GENA LastChange subscription (or a GetTransportInfo poll) fed back into the state machine, so device-side pause/skip is reflected. Prefer SetNextAVTransportURI for gapless where supported (tideway never used it). Version-agnostic service matching. Crate: rupnp.
