---
id: canon-bc84
title: Accurate cast position from MEDIA_STATUS (reconcile frames-fed vs reported)
type: task
priority: 2
created: '2026-09-23T00:24:18Z'
updated: '2026-09-23T03:19:45Z'
parent: canon-7718
labels:
- sink
---

v1 drives position from frames fed to the encoder, which leads actual Cast playback by the Cast buffer (~1-3s). Reconcile the FrameClock to the Cast MEDIA_STATUS reported position (seek clock on each status, free-run by wall-time between). Needs a wall-time clock advance on the network path.

---
▸ 2026-09-23T03:15:40Z [Joel Webber]
SEQUENCING DECISION (user instinct, endorsed): do this BEFORE canon-685a (DLNA). It is not a Cast detail — it is 'where does playback position come from when a NETWORK renderer is playing', which is shared infrastructure every network sink needs. Settle the authority once and DLNA inherits it; do DLNA first and we end up reconciling two half-baked position models. (User: pulling stream position from the wrong place was a source of unending pain in tideway — same class of bug.)
CURRENT STATE: on the network path the engine calls clock.advance(frames_fed), so position tracks what we have ENCODED AND SENT, which leads the audio the renderer is actually playing by its buffer depth (~1-3s). Worse, it is open-loop: nothing ever corrects drift.
WHAT WE ALREADY HAVE: the Cast status poll returns StatusEntry.current_time (observed live, e.g. current_time: Some(76.34827)) every ~500ms. DLNA's equivalent is AVTransport GetPositionInfo/RelTime (plus GENA events). So both protocols can report authoritative position — the seam should take 'renderer-reported position' generically, not Cast-specific.
DESIGN SHAPE TO SETTLE:
- FrameClock is frames+epoch+sample_rate and is designed to be advanced by the RT callback. There is no RT callback on the network path, so it needs a wall-time advance mode (free-run between reports) plus correction from the renderer's reported position.
- Do NOT hard-seek on every report: at 2/sec that would visibly jitter the position. Prefer a drift threshold (only correct when |reported - derived| exceeds e.g. 500ms) or slew, so normal playback is smooth and only real divergence snaps.
- Watch the epoch contract: seek() bumps epoch (clients treat it as a discontinuity). A routine position correction is NOT a user-visible discontinuity, so it likely needs a distinct 'reconcile' path that does not bump epoch, or clients will see constant seeks.
- Carry it as a new generic event (e.g. CastEvent::Position(ms) -> a controller-level 'renderer position' input) so canon-685a plugs into the same path.
