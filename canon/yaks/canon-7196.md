---
id: canon-7196
title: Headless player mode for the always-on box
status: hairy
type: feature
priority: 2
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T22:14:41Z'
parent: canon-88a7
depends_on:
- canon-1ad8
source: arch
---

After the split, whatever drives a speaker must stay running: closing a laptop that drives Kitchen stops Kitchen. The always-on box therefore runs the server plus a headless player (executor role only, no UI), controlled by any UI that names it. It must work with no local audio device at all (the founding constraint, docs/arch.md section 1).

---
▸ 2026-10-17T22:14:41Z [delta:The Great Decoupling]

Implemented as `canon player --headless` (canon-1ad8's binary): executor only, reconnects with backoff 1s→30s when the server goes. Verified live against a test server (reconnect attempts logged). Not verified: running on a box with no audio device (renderers only) — the controller already handles 'no local output' as before, but needs the real box.
