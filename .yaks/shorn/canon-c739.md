---
id: canon-c739
title: 'Connections in core, Tidal only: capabilities, connectors, per-connection credentials, routing'
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:43:06Z'
parent: canon-b3a5
labels:
- arch
- tidal
verify: cargo test -p canon-core source && cargo test -p canon-tidal connector && cargo test -p canon-api
---

Step 1 of docs/connections.md section 8. Capability/Connection/Connector/Method/LoginFlow in canon-core replacing ServiceSession; Tidal connector with methods tidal.pkce (Stream lossless/hi-res + catalog/library/recs) and tidal.device (catalog/library/recs, no stream); per-connection credential files (tidal.json migrated by its is_pkce); API ops services, connect, connect_complete, connections, disconnect (login_* retired); Sources routes by capability and the six Tidal defaults become the connection with Catalog; Error::NotEntitled with a hint. Fixes the device-code-overwrites-PKCE bug.

---
▸ 2026-09-24T21:42:51Z [Joel Webber]
Built: canon-core connection.rs (Capability, Capabilities, Method, FlowKind, LoginFlow, Health, ConnectionInfo, Connector trait) replacing ServiceSession; Error::NotEntitled {service, capability, hint}; Sources routes bindings/catalogs through connectors by capability (opening needs Stream, describing only Catalog; a service with no connector is passed over; a connector without the capability yields NotEntitled). canon-tidal TidalConnector: tidal.pkce (streams, HiRes) and tidal.device (no stream), one TidalSession + credential file per method (tidal.pkce.json / tidal.device.json), legacy tidal.json moved to its method (a newer legacy file wins, since an older canon still running refreshes into it), disconnect = forget. API: services, connect, connect_complete, disconnect replace login_begin/login_poll/account; the Tidal defaults are now the first service that can browse. Daemon on the connector (serve, login, resolve, tidal-get); canon control services / connect / disconnect.

---
▸ 2026-09-24T21:42:51Z [Joel Webber]
Live 2026-09-24: first serve moved the real tidal.json to tidal.pkce.json ("moved ... tidal.json to ... tidal.pkce.json", "tidal can stream", "tidal can browse"). canon resolve 33348478 through the connector: Flac LOSSLESS 44100 Hz, 59 segments. Empty --state-dir daemon on :7399: services shows tidal.pkce (streaming up to hi_res) and tidal.device (no streaming), both not signed in; enqueue -> "tidal cant browse: sign in to Tidal (tidal.pkce, or tidal.device for browsing only)"; search -> "nothing can be browsed: connect a service first"; connect tidal.pkce -> the login.tidal.com authorize URL; connect tidal.nope -> "no login method tidal.nope". Lesson recorded in AGENTS.md: Joel runs his own daemon on :7345; the first live attempt connected to it and a bare pkill would have killed it; test daemons now use their own port.

---
▸ 2026-09-24T21:43:06Z [Joel Webber]
verify: `cargo test -p canon-core source && cargo test -p canon-tidal connector && cargo test -p canon-api` -> PASS (exit 0)
