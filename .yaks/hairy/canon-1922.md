---
id: canon-1922
title: Break a flow stream when a renderer reconnects mid-stream
type: task
priority: 3
created: '2026-09-23T20:24:31Z'
updated: '2026-09-23T20:24:31Z'
parent: canon-77f8
labels:
- arch
- sink
- network
---

The decision in canon-77f8 (item 4): a renderer that reconnects mid-stream is served the live edge, so the audio it skipped silently offsets every later join boundary. We observe the reconnect at our own stream server, and a fresh load makes the timeline correct by construction.

PARKED (2026-09-23) until one is observed: no mid-stream reconnect has appeared in any on-metal run (Cast or DLNA; every "consumer connected" was at a load: Cast 1 per load, Rygel probe+fetch 2 per load), and it cannot be provoked from our side. Ending a body server-side reads to the renderer as end of stream, which is exactly how flow ends, not as a dropped connection it would retry. Shipping it without an observation would mean shearing without evidence.

DESIGN when it is picked up: StreamBroadcaster counts subscriptions. The controller records a baseline for the current load the first time the renderer reports Playing for it (so start-of-stream probe+fetch pairs are normal). The watchdog tick breaks when the newest stream subscriptions exceed the baseline, via a new EngineEvent (e.g. StreamRejoined) tagged with the run, which the actor answers with restart_here(). Rate-limit it (at most one break per session per minute, else warn and re-baseline), so a device that always reconnects cannot loop.
CHEAP FIRST STEP: a WARN log when a stream gains a consumer after its renderer has reported Playing, so a real occurrence shows up in serve.log with a timestamp and device.
