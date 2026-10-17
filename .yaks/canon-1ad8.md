---
id: canon-1ad8
title: 'canon player: the TUI plus local and LAN output in one binary'
status: shaving
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T22:03:46Z'
parent: canon-88a7
depends_on:
- canon-806d
- canon-040e
- canon-5ce4
- canon-65d2
source: arch
---

The first player: the TUI's UI (controller role) and today's `PlaybackController` + `canon-audio` + `canon-sink` (executor role) in one process, talking to a remote server. It plays to local audio and drives Cast/DLNA itself, so the firewall, signing, and `--lan-port` rules in AGENTS.md move with it. Fetches its audio from the server's FLAC endpoint and retries with backoff, giving up with `EngineEvent::Failed`.

Shorn only after a real session: play, seek, pause, sink switch to Tunes, and a dropped server connection.

---
▸ 2026-10-17T22:03:46Z [delta:The Great Decoupling]

moved: hairy -> shaving
