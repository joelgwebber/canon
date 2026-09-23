---
id: canon-a70f
title: 'Wire cast sinks into the controller: SelectSink + fail back to local'
type: task
priority: 2
created: '2026-09-23T02:03:10Z'
updated: '2026-09-23T02:03:10Z'
parent: canon-7718
labels:
- sink,api
---

canon-dde4 proved the Cast path via the 'canon cast' harness; now route it through the daemon proper. Command::SelectSink(<discovered id>) should: build a CastSink, start the LAN stream server on the chosen iface, restart the current track on Output::Network at the current position (single-active-output, per the dde4 decision), and feed CastEvents INTO PlaybackController's state actor (Playing/Paused/Ended/Superseded/Failed -> player transitions, incl. auto-advance on Ended). Watch SinkHealth and fail back to Output::Local on Failed (EngineEvent::SinkFailed already exists). Also expose the discovery snapshot over the WS API so clients can list/select sinks.
