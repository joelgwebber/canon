---
id: canon-52fb
title: 'Spike: Spotify audio via librespot as a Source'
type: task
priority: 3
created: '2026-09-24T21:28:12Z'
updated: '2026-09-24T21:28:12Z'
parent: canon-b3a5
labels:
- spotify
- audio
---

Step 5. Prototype librespot (OAuth, Premium) opening one Spotify track as a canon Source (Ogg Vorbis decode through the existing engine), nothing wired into playback. Test carefully: audio-key refusals reported since Nov 2025 (librespot #1649). Fallback if it fails: control Connect speakers via the Web API player endpoints (docs/connections.md section 6).
