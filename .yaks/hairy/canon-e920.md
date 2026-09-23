---
id: canon-e920
title: Per-track stream paths on the LAN stream server (gapless groundwork)
type: task
priority: 2
created: '2026-09-23T14:50:48Z'
updated: '2026-09-23T14:50:48Z'
parent: canon-7718
labels:
- arch
- sink
- network
---

From canon-ba30 (E). A network session serves one URL (/stream.flac) and each track installs a fresh FlacTap over one broadcaster, so there is never a second URL to hand a renderer. DLNA SetNextAVTransportURI (and any gapless/preload on Cast queues) needs the next track to be addressable while the current one plays: serve /stream/<generation>.flac, one broadcaster per path, with header replay unchanged. Keep consumers() liveness per session (sum, or the current path). Do this alongside canon-685a if gapless is attempted there.
