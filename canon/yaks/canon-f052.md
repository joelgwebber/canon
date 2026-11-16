---
id: canon-f052
title: canon serve without outputs of its own, for servers whose players play elsewhere
status: hairy
type: feature
priority: 3
created: '2026-10-18T15:38:15Z'
updated: '2026-10-18T15:38:15Z'
parent: canon-88a7
source: arch
---

Every player's first executor is the server's own controller, so `canon serve` always runs discovery and holds `--lan-port`, even when every player is a remote `canon player`. On one box that collides with the player (see the SSDP fallback yak); on a server box it's discovery traffic nothing uses. A flag or setting (e.g. `--no-output`) that skips discovery and the built-in executor: a player with no remote executor then fails a start at once, as it already does after its executor leaves. Decide whether this is a flag, a setting, or the default once a server has paired players.
