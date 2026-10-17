---
id: canon-e329
title: Extract a canon-protocol crate with no sqlite in it
status: hairy
type: task
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T19:15:46Z'
parent: canon-88a7
source: arch
---

Clients (the TUI today, players and native apps later) link `canon-library`, and so sqlite, only for wire types: `canon-tui` imports `ItemRef`/`EntityKind` from it (crates/canon-tui/src/app.rs), and `canon-api`'s `protocol.rs` pulls in library view types.

Move every type that crosses the WebSocket (client/server messages, the library views, `PlayerSnapshot`, `Command`, `SinkInfo`, ...) into a `canon-protocol` crate that depends on serde and little else, so a client builds without the library. The JSON schema it defines is the contract a non-Rust player will implement against.

Done when `cargo tree -p canon-tui` shows no rusqlite and the green bar passes. A pure refactor: no wire change.
