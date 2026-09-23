---
id: canon-d441
title: 'Settings: deployment knobs in the file? And client type codegen (ts-rs vs schemars)'
type: task
priority: 3
created: '2026-09-23T20:06:40Z'
updated: '2026-09-23T20:06:47Z'
labels:
- architecture
needs: human
---

Split out of canon-f04a when it was done minimally (2026-09-23) for per-output flow/standard mode. Two open questions, both needing a human:
(1) Should the daemon deployment knobs (bind; quality) live in the settings file too, for a hosted Linux box, or are systemd flags fine? state_dir cannot: the file lives in it. If yes, they join the same Settings struct (no second schema), with explicit side effects on change (bind needs a restart).
(2) Generated client types: ts-rs or schemars? Only matters once a real UI consumes them; it shapes the derives on Settings.

---
▸ 2026-09-23T20:06:47Z [Joel Webber]
Two decisions for Joel: deployment knobs in the settings file (yes/no), and ts-rs vs schemars for client types.
