---
id: canon-d441
title: 'Settings: deployment knobs in the file? And client type codegen (ts-rs vs schemars)'
type: task
priority: 3
created: '2026-09-23T20:06:40Z'
updated: '2026-09-27T17:17:09Z'
labels:
- architecture
---

Split out of canon-f04a when it was done minimally (2026-09-23) for per-output flow/standard mode. Two open questions, both needing a human:
(1) Should the daemon deployment knobs (bind; quality) live in the settings file too, for a hosted Linux box, or are systemd flags fine? state_dir cannot: the file lives in it. If yes, they join the same Settings struct (no second schema), with explicit side effects on change (bind needs a restart).
(2) Generated client types: ts-rs or schemars? Only matters once a real UI consumes them; it shapes the derives on Settings.

---
▸ 2026-09-23T20:06:47Z [Joel Webber]
Two decisions for Joel: deployment knobs in the settings file (yes/no), and ts-rs vs schemars for client types.

---
▸ 2026-09-27T17:17:09Z [Joel Webber]
Joel (2026-09-27): (1) Deployment knobs stay out of the settings file: bind and state_dir remain flags/env (clap env already on; state_dir can't live in a file inside itself; bind needs a restart). Quality is a listening preference, not deployment, so it moves into Settings next to streaming.order. (2) schemars over ts-rs: JSON Schema serves every client language and the MCP tool layer (canon-c67f) needs it anyway, so one set of derives serves both. Deferred until MCP or a real UI needs it.

---
▸ 2026-09-27T17:17:09Z [Joel Webber]
Remaining work: move quality into Settings; schemars derives land with canon-c67f.
