---
id: canon-c739
title: 'Connections in core, Tidal only: capabilities, connectors, per-connection credentials, routing'
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:28:11Z'
parent: canon-b3a5
labels:
- arch
- tidal
---

Step 1 of docs/connections.md section 8. Capability/Connection/Connector/Method/LoginFlow in canon-core replacing ServiceSession; Tidal connector with methods tidal.pkce (Stream lossless/hi-res + catalog/library/recs) and tidal.device (catalog/library/recs, no stream); per-connection credential files (tidal.json migrated by its is_pkce); API ops services, connect, connect_complete, connections, disconnect (login_* retired); Sources routes by capability and the six Tidal defaults become the connection with Catalog; Error::NotEntitled with a hint. Fixes the device-code-overwrites-PKCE bug.
