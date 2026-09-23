---
id: canon-333e
title: Realtime bus (cross-device pause) + play reporting
type: task
priority: 2
created: '2026-09-22T02:01:38Z'
updated: '2026-09-23T04:32:42Z'
labels:
- tidal
- network
---

Receive half: the 'Pushkin' websocket (mint a per-connect token via POST /v1/rt/connect, then wss://pushkin-v2; PRIVILEGED_SESSION_NOTIFICATION -> pause local; exponential backoff; RECONNECT frame reopens). Send half: post our own playback events to ec.tidal.com/api/event-batch (SQS-form, masquerade as Android/androidAuto, identity from the access-token JWT) so we appear in Recently Played and pause other devices. Crate: tokio-tungstenite. Expose a loopback /realtime/status diagnostic like tideway's.

---
▸ 2026-09-23T04:32:42Z [Joel Webber]
Hoisted out of canon-94cc (Tidal source layer). It sat there because it speaks Tidal's API, but the source layer's deliverable is 'get bytes and metadata out of Tidal', and this is not that. It is the only piece in that tree that writes TO Tidal (play reporting to ec.tidal.com so we appear in Recently Played), and its receive half reaches into the player to pause local playback on a privileged-session notification. That makes it a multi-device presence feature that happens to use Tidal's transport, not part of sourcing music -- and keeping a completed epic open for it misread how much of the Tidal work was actually left.

Note for whoever picks it up: the pause-local half is a player input, so it should arrive as an EngineEvent and not as Command::Pause, for the same reason renderer reports do (canon-bc84). 'Another device took over' is device status, not user intent.
