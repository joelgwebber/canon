---
id: canon-587a
title: Tag renderer events with the generation of the LOAD they belong to
type: bug
priority: 2
created: '2026-09-23T14:50:48Z'
updated: '2026-09-23T16:13:26Z'
parent: canon-7718
depends_on:
- canon-583a
labels:
- arch
- sink
verify: cargo test -p canon-daemon
---

From canon-ba30 (D). run_cast_events tags a device Ended with inner.generation read at *receipt*. If a user Next bumps the generation just before the old media Finished status arrives, the stale Ended is attributed to the new track and auto-advance skips one extra. Record the generation when a LOAD is accepted (Cast: media_session_id -> generation; DLNA: the URI we set) and tag events with it, so on_engine_event stale-filtering works for renderer events the same way it does for engine events. Likely easiest as part of the canon-583a RendererEvent shape. Unit-test the race: Ended for generation N arriving after the bump to N+1 must be ignored.

---
▸ 2026-09-23T16:13:24Z [Joel Webber]
Fixed with canon-44f4. Sink::load returns a LoadId, and reports arrive as RendererReport { load, event }. Cast attributes each report to the load whose media session it describes: current moves only once the receiver has accepted the next LOAD, and the EdgeFilter resets per load (test a_new_load_rearms_every_edge). NetworkSession.current = (LoadId, generation) is set when a load is issued and cleared on stop. controller::attribute forwards State/Position/Ended only when the load is current and its generation is still live. Superseded/Failed are session-level and honoured regardless, so a takeover mid-track-change is not dropped. Unit tests: a_report_about_the_live_load_counts, a_finish_arriving_after_a_skip_is_stale, a_report_about_a_superseded_load_is_stale, nothing_counts_with_nothing_loaded. On metal: stop on track 1 of 2 left queue=0/2 idle for 10s. The Cancelled the device reports after our stop is no longer read as Ended (see canon-44f4 evidence).

---
▸ 2026-09-23T16:13:26Z [Joel Webber]
verify: `cargo test -p canon-daemon` -> PASS (exit 0)
