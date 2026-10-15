---
id: canon-26c1
title: Keep reconnecting a dropped renderer for a while, holding position
status: shaving
type: bug
priority: 2
created: '2026-10-04T16:24:10Z'
updated: '2026-10-04T16:51:14Z'
parent: canon-5303
depends_on:
- canon-9bce
labels:
- sink
- cast
---

reconnect() gets one select_sink, i.e. connect_settled's 3 attempts 1s apart (~2.3s). canon-5303's drop was Tunes closing its Cast session and refusing :8009 for at least that long, i.e. its Cast service restarting. Keep trying for ~30s, and meanwhile hold the listener's position (the renderer clock otherwise extrapolates through the outage, and the restart lands that far ahead) and stop feeding a stream nothing consumes. Needs canon-9bce, or a speaker that ignores TCP makes each attempt last minutes.

---
▸ 2026-10-04T16:51:14Z [Joel Webber]
Built 2026-10-04: reconnect() releases the dead session, stops the network feed (nothing consumes it), holds the listener's place (RendererState::Buffering to the current playback, when playing, so the renderer clock stops), then retries select_sink every 2s for RECONNECT_FOR (30s), checking before each attempt that the listener hasn't chosen another output. A successful reconnect restarts from the held position and restarts RECONNECT_WINDOW from when it came back. On hardware (idle session on Library display, scripts/netfault.c): a drop reconnected on attempt 1 ('reconnected attempts=1'); drop + refuse retried every ~4s ('not back yet … trying again', 8 attempts) and gave up at 30s with the reason. Still owed before shearing: the held position across a reconnect while really playing, which needs a speaker able to fetch a test daemon's stream (ufw drops 7347 on this box; Joel to allow it).
