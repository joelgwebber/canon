---
id: canon-9487
title: Snapshot + seq + delta stream to clients
type: task
priority: 2
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T13:18:32Z'
parent: canon-e284
labels:
- state
- api
verify: cargo test -p canon-api
---

Clients get an immediate full snapshot on connect, then deltas; reconcile by seq; terminal states (ended/error) always forwarded. Generalizes tideway's reset-then-delta download broker to player + queue. Prefer emitting (position, timestamp, rate) so clients interpolate smoothly without a 4Hz server poll.

---
▸ 2026-09-22T13:17:47Z [Joel Webber]
verify: `cargo test -p canon-api` -> PASS (exit 0)

---
▸ 2026-09-22T13:17:53Z [Joel Webber]
verify: `cargo test -p canon-api` -> PASS (exit 0)

---
▸ 2026-09-22T13:18:11Z [Joel Webber]
Shorn: canon-api streams the full seq-stamped PlayerSnapshot on connect and on every change; clients reconcile by seq and interpolate with rate (no 4Hz poll). Decision: ship full snapshots, not field-level deltas — the struct is ~10 fields and self-consistent, and the seq contract leaves room to add true deltas later without a client change. Evidence: cargo test -p canon-api (control_plane end-to-end, PASS).
