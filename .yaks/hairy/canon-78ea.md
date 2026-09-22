---
id: canon-78ea
title: Canonical entity model + persistence (sqlite)
type: task
priority: 2
created: '2026-09-22T02:01:39Z'
updated: '2026-09-22T02:43:39Z'
parent: canon-4185
labels:
- library
---

canon-native Track/Release/Artist entities with stable canon IDs, persisted (rusqlite/sqlx) — not tideway's rebuilt-every-launch in-memory map. Each entity carries external references. The identity/join-key strategy (canon-native + ISRC vs adopting MusicBrainz MBIDs) is the open question on this yak.

---
▸ 2026-09-22T02:01:40Z [claude]
Library identity: canon-native IDs with ISRC as the join key (simple, self-contained) vs adopting MusicBrainz recording/release MBIDs as the canonical identity (richer cross-service matching + metadata, but a real external dependency and modelling cost). Which way should the entity model lean?

---
▸ 2026-09-22T02:43:39Z [Joel Webber]
I'd really like to be able to match tracks across services and local music, so let's bite the bullet and take on MBIDs.
