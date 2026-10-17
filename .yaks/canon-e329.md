---
id: canon-e329
title: Extract a canon-protocol crate with no sqlite in it
status: shorn
type: task
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-17T19:28:15Z'
parent: canon-88a7
source: arch
verify: test "$(cargo tree -p canon-tui -e normal | grep -c rusqlite)" = 0 && cargo test -p canon-protocol -p canon-tui -p canon-api -p canon-library
---

Clients (the TUI today, players and native apps later) link `canon-library`, and so sqlite, only for wire types: `canon-tui` imports `ItemRef`/`EntityKind` from it (crates/canon-tui/src/app.rs), and `canon-api`'s `protocol.rs` pulls in library view types.

Move every type that crosses the WebSocket (client/server messages, the library views, `PlayerSnapshot`, `Command`, `SinkInfo`, ...) into a `canon-protocol` crate that depends on serde and little else, so a client builds without the library. The JSON schema it defines is the contract a non-Rust player will implement against.

Done when `cargo tree -p canon-tui` shows no rusqlite and the green bar passes. A pure refactor: no wire change.

---
▸ 2026-10-17T19:21:43Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T19:28:15Z [delta:The Great Decoupling]

verify: `test "$(cargo tree -p canon-tui -e normal | grep -c rusqlite)" = 0 && cargo test -p canon-protocol -p canon-tui -p canon-api -p canon-library` -> PASS (exit 0)

---
▸ 2026-10-17T19:28:15Z [delta:The Great Decoupling]

Evidence (2026-10-17):
- `cargo tree -p canon-tui -e normal | grep -c rusqlite` → 0 (was linked via canon-library). `cargo tree -p canon-protocol --depth 1`: canon-core, serde only.
- Green bar: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets` 0 warnings; `cargo test --workspace` all ok (canon-protocol 6 passed: the message round-trip tests and the ItemRef::from_url tests moved with their types); `cargo build --workspace` ok.
- No wire change: the TUI's insta frame snapshots pass unchanged. Live: this build's `canon serve --bind 127.0.0.1:7399 --lan-port 7347` (shared state dir; no schema change) + `canon tui --headless`: Library tab rendered `Tracks (256)` with saved rows from the real library, i.e. LibraryPage/TrackView decoded off the wire; `canon control` status/queue answered.

Decision: moved, not re-exported. `canon-api::protocol` and `canon-library::{view, EntityKind, ItemRef}` are gone rather than kept as aliases, so there is one path to each wire type. `TrackView::mark` (needs `Sources`) became a private `mark_one` in canon-library, since the protocol crate must not see `Sources`. `EntityKind::as_str/parse` went from pub(crate) to pub: the store keys on them.

---
▸ 2026-10-17T19:28:15Z [delta:The Great Decoupling]

moved: shaving -> shorn
