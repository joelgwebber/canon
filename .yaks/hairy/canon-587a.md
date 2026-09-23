---
id: canon-587a
title: Tag renderer events with the generation of the LOAD they belong to
type: bug
priority: 2
created: '2026-09-23T14:50:48Z'
updated: '2026-09-23T14:50:48Z'
parent: canon-7718
depends_on:
- canon-583a
labels:
- arch
- sink
---

From canon-ba30 (D). run_cast_events tags a device Ended with inner.generation read at *receipt*. If a user Next bumps the generation just before the old media Finished status arrives, the stale Ended is attributed to the new track and auto-advance skips one extra. Record the generation when a LOAD is accepted (Cast: media_session_id -> generation; DLNA: the URI we set) and tag events with it, so on_engine_event stale-filtering works for renderer events the same way it does for engine events. Likely easiest as part of the canon-583a RendererEvent shape. Unit-test the race: Ended for generation N arriving after the bump to N+1 must be ignored.
