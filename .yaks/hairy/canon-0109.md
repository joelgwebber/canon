---
id: canon-0109
title: On-metal checks owed while LAN discovery is blocked (2026-09-23)
type: task
priority: 2
created: '2026-09-23T21:49:21Z'
updated: '2026-09-23T21:49:21Z'
labels:
- sink
- network
needs: human
---

At ~17:40 on 2026-09-23 canon devices stopped finding any renderer (Tunes answers ping at 192.168.0.205; the last known-good commit 5532311, rebuilt at the same path, finds nothing either), so macOS Local Network permission for target/debug/canon is the suspect, not the code. Checks owed once discovery is back:
- canon-edc7: flow-mode superseded join on Tunes (play 55391795 33348478, seek 3:42, playnext 55391796 within ~5s: after Brain Damage, Eclipse must play, not Army of Me).
- anything else shorn in the meantime whose note says 'network path unit-tested only'.

---
▸ 2026-09-23T21:49:21Z [Joel Webber]
Could you check System Settings > Privacy & Security > Local Network (and re-grant the terminal / canon), or otherwise tell me what changed on the Mac's network? canon sees no mDNS/SSDP at all since ~17:40.
