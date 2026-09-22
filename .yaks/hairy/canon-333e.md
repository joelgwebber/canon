---
id: canon-333e
title: Realtime bus (cross-device pause) + play reporting
type: task
priority: 2
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T02:01:38Z'
parent: canon-94cc
labels:
- tidal
- network
---

Receive half: the 'Pushkin' websocket (mint a per-connect token via POST /v1/rt/connect, then wss://pushkin-v2; PRIVILEGED_SESSION_NOTIFICATION -> pause local; exponential backoff; RECONNECT frame reopens). Send half: post our own playback events to ec.tidal.com/api/event-batch (SQS-form, masquerade as Android/androidAuto, identity from the access-token JWT) so we appear in Recently Played and pause other devices. Crate: tokio-tungstenite. Expose a loopback /realtime/status diagnostic like tideway's.
