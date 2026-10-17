---
id: canon-7196
title: Headless player mode for the always-on box
status: hairy
type: feature
priority: 2
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T19:15:56Z'
parent: canon-88a7
depends_on:
- canon-1ad8
source: arch
---

After the split, whatever drives a speaker must stay running: closing a laptop that drives Kitchen stops Kitchen. The always-on box therefore runs the server plus a headless player (executor role only, no UI), controlled by any UI that names it. It must work with no local audio device at all (the founding constraint, docs/arch.md section 1).
