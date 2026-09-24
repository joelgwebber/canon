---
id: canon-b391
title: Match a track onto another service by ISRC and bind it
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:28:11Z'
parent: canon-880e
labels:
- library
- tidal
---

Step 3a of docs/connections.md. Catalog gains a lookup by ISRC (Tidal: check what v1 offers with canon tidal-get, else search plus an ISRC check); Library::match_onto(entity, service) finds the service's recording with the entity's ISRC, ingests and binds it (provenance isrc), and reports no-match honestly. Independent of the connections work.
