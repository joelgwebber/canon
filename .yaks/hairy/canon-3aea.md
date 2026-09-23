---
id: canon-3aea
title: DLNA gapless via SetNextAVTransportURI
type: task
priority: 3
created: '2026-09-23T16:55:41Z'
updated: '2026-09-23T16:55:41Z'
parent: canon-7718
depends_on:
- canon-685a
labels:
- sink
- network
---

From canon-685a. Per-load streams (canon-e920) make the next track addressable while the current one plays, so the groundwork is there. Resolve and open the next queue entry about 10-20s before the end, SetNextAVTransportURI it, and treat the renderer moving to the next TrackURI as the track boundary: attribute reports to the next load without a fresh SetAVTransportURI+Play. Needs a way for the controller to pre-resolve the next track (currently each track is resolved at play time). Check first that the renderer AVTransport SCPD advertises SetNextAVTransportURI. The LS50 is AVTransport:2 and likely does.
