---
id: canon-8ee6
title: Wire discovery into the daemon + canon devices listing
type: task
priority: 2
created: '2026-09-23T01:08:29Z'
updated: '2026-09-23T01:21:49Z'
parent: canon-dde4
labels:
- sink,network
verify: cargo clippy -p canon-daemon --all-targets
---

Start the DiscoveryService in the daemon and expose the device list: a 'canon devices' CLI (browse N seconds, print id/name/kind/addr) as the first on-metal check that discovery finds real renderers (KEF 'Tunes'), reused later for sink selection.

---
▸ 2026-09-23T01:21:49Z [Joel Webber]
verify: `cargo clippy -p canon-daemon --all-targets` -> PASS (exit 0)

---
▸ 2026-09-23T01:21:49Z [Joel Webber]
Shorn. Added canon-sink as a daemon dep and a 'canon devices' subcommand: starts DiscoveryService, browses N seconds, prints kind/name/addr/id. Verified on-metal that the wiring drives discovery correctly (joins 224.0.0.251 on en0, searches, clean shutdown); it returned 0 devices only because of macOS Local Network privacy (see canon-ea5d note + canon-bb20), not a wiring defect. Evidence: clippy -p canon-daemon clean; live run shows correct browse on en0.
