---
id: canon-5100
title: Warn when DLNA search falls back from --lan-port
status: hairy
type: bug
priority: 2
created: '2026-10-18T15:38:15Z'
updated: '2026-10-18T15:38:15Z'
parent: canon-88a7
source: arch
---

`canon_sink::ssdp::search` falls back to an OS-picked port when `--lan-port` is taken, logging only at debug. A firewall then drops the speakers' unicast answers and DLNA devices silently never appear. Hit live: `canon serve` and `canon player` on one machine both defaulting to 7346 (canon-88a7 session, 2026-10-18). The stream server already warns in the same case; SSDP should warn the same way, once, naming the port it fell back to and the ufw command (see canon-6227's firewall check).
