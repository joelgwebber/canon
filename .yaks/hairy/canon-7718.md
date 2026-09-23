---
id: canon-7718
title: Sink abstraction & network renderers
type: feature
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-23T20:35:17Z'
labels:
- sink
---

Local and network outputs behind one Sink trait, with resilient discovery and clean session lifecycle. This is where tideway was flakiest (the whole tide-4000.x and tide-6fd0 families), so get the shapes right from the start; specific protocol fiddliness (OpenHome, Tidal Connect, gapless) can come later.

---
▸ 2026-09-22T21:58:31Z [Joel Webber]
DECISION (user-approved): Chromecast first, then DLNA on the same foundation. Rationale: uniform receiver spec vs DLNA device-zoo; push MEDIA_STATUS vs flaky GENA; mDNS vs SSDP; rust_cast head start; forces building the shared LAN stream server DLNA reuses. Build order: cf48 (Sink trait+lifecycle) -> 21f7 (LAN FLAC server) -> ea5d (discovery supervisor) -> dde4 (Cast) -> 685a (DLNA). No de-risk spike first; go straight at the foundation. User OK'd parallelizing via yak-worktree where write scopes are disjoint.
