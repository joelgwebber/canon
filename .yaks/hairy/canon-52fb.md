---
id: canon-52fb
title: 'Spike: Spotify audio via librespot as a Source'
type: task
priority: 3
created: '2026-09-24T21:28:12Z'
updated: '2026-09-24T21:59:07Z'
parent: canon-b3a5
labels:
- spotify
- audio
needs: human
---

Step 5. Prototype librespot (OAuth, Premium) opening one Spotify track as a canon Source (Ogg Vorbis decode through the existing engine), nothing wired into playback. Test carefully: audio-key refusals reported since Nov 2025 (librespot #1649). Fallback if it fails: control Connect speakers via the Web API player endpoints (docs/connections.md section 6).

---
▸ 2026-09-24T21:59:07Z [Joel Webber]
Before the librespot spike: is your Spotify account Premium (librespot's audio needs it, and so does owning a dev-mode Web API app)? And OK to sign your account in through librespot (reverse-engineered; Spotify's terms likely forbid it; audio-key refusals reported since Nov 2025)? If not, the fallback is controlling Connect speakers through the Web API player endpoints.
