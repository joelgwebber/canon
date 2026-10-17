---
id: canon-65d2
title: 'Server auth: player tokens and signed stream URLs'
status: shaving
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T20:40:51Z'
parent: canon-88a7
depends_on:
- canon-973d
source: arch
---

The API binds 127.0.0.1:7345 with no auth. Once players reach the server over the LAN (and later the internet) it needs auth, designed in now and used on the LAN too, so that internet exposure later means TLS in front, not a second code path.

- Pairing issues a long-lived player token; the token is also the player's identity (the per-player actor yak keys on it).
- The WS handshake carries the token; stream URLs carry short-lived signed tokens, since native players want a bare URL.
- A LAN bind option, and the canon-6227 firewall startup check applied to the server's ports.
- The player's own speaker-facing stream server stays unauthenticated on its LAN: speakers can't auth.

---
▸ 2026-10-17T20:40:51Z [delta:The Great Decoupling]

moved: hairy -> shaving
