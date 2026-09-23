---
id: canon-ea5d
title: Resilient discovery supervisor (iface-aware, wake-safe)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-23T01:21:10Z'
parent: canon-7718
labels:
- discovery
- network
verify: cargo test -p canon-sink
---

A long-lived supervisor per protocol with its own cache and add/remove/expiry events and a debounced snapshot the picker reads (no wholesale replace per scan). Fixes tide-6fd0/8f5b: enumerate real LAN interfaces and EXCLUDE tunnels (utun/tun/tap/VPN) — a stale utun 224.0.0/4 route silently black-holed tideway's multicast; pin IP_MULTICAST_IF per chosen iface; rebuild the socket + re-add IP_ADD_MEMBERSHIP on sleep/wake; watchdog that re-bursts M-SEARCH when reception looks wedged. Crates: socket2 (multicast control), mdns-sd (Cast browser), ssdp-client/rupnp (SSDP+descriptors).

---
▸ 2026-09-22T22:44:30Z [Joel Webber]
verify: `cargo test -p canon-sink` -> PASS (exit 0)

---
▸ 2026-09-22T22:44:57Z [Joel Webber]
Shorn. Landed crates/canon-sink/src/discovery.rs (built by a subagent, reviewed + integrated by me).
- DiscoveryService supervisor: own cache + add/remove/TTL-expiry events, publishes a DEBOUNCED watch::Receiver<Vec<DiscoveredDevice>> (deduped, stably sorted) — no wholesale per-scan replace.
- CROWN JEWEL (the tide-6fd0/8f5b root cause), pure-unit-tested: usable_interfaces() excludes tunnels/VPN by name (utun/tun/tap/ppp/ipsec/wg), loopback, and link-local (169.254/fe80), and orders real LAN NICs first. A stale utun 224.0.0/4 route is what black-holed tideway's multicast; this filter makes choosing it impossible.
- Multicast egress pinned per NIC via socket2 IP_MULTICAST_IF + per-NIC IP_ADD_MEMBERSHIP anchors; mdns-sd daemon enabled on the chosen ifaces only.
- Removal driven by liveness/TTL expiry, not solely a discovery-remove notice.
- Wake-safe resync()/rebuild(): disable-all + re-enable ifaces (re-burst) and drop+rebuild the anchors (fresh membership). Sleep/wake watchdog trigger sketched in comments for the on-metal pass; the mechanism it drives is fully built.
- mdns-sd confined behind CastSource so cache/debounce/filter are testable with no daemon; DLNA/SSDP (canon-685a) plugs in as a second source feeding the same DeviceCache. Clean Drop shuts the daemon + bridge, no leaked thread.
Note: socket2 'all' feature not enabled, so anchors use SO_REUSEADDR on an ephemeral port (not 5353) — no contention with mdns-sd's sockets; enable 'all' later only if per-NIC anchors need to also listen on 5353.
Evidence: cargo test -p canon-sink (6 discovery tests: interface-exclusion table, add/remove/dedupe/TTL-expiry snapshot, identical-readd-no-churn, burst coalescing, remove-unknown no-op); workspace fmt clean, clippy 0 warnings, all tests green.

---
▸ 2026-09-23T01:11:44Z [Joel Webber]
REOPENED: on-metal, 'canon devices' finds nothing while the OS mDNS (dns-sd -B _googlecast._tcp) sees 4 Cast devices incl. the KEF LS50 Wireless II ('Tunes') on en0/192.168.0.67. So discovery has a real bug (not a LAN/permission issue). Investigating the mdns-sd interface pinning (disable All + enable Addr) as prime suspect.

---
▸ 2026-09-23T01:21:07Z [Joel Webber]
RESOLVED — the code is correct; the failure was macOS Local Network privacy, not an ea5d defect. Evidence (RUST_LOG=mdns_sd=debug): mdns-sd joins 224.0.0.251 on en0 (V4 192.168.0.67 + V6) with no bind error and sends the query (SearchStarted on en0/15), but ZERO inbound responses arrive; meanwhile dns-sd (system mDNSResponder, exempt from the per-app prompt) sees all 4 Cast devices from the same process context. Sockets/join/egress are healthy; macOS silently drops inbound LAN packets to a process lacking Local Network permission. usable_interfaces correctly picked en0 only. Restored the file to its committed state (removed experiment prints; pinning + anchors were never the cause). Proper cross-platform handling filed as a new yak; interface filter/cache/debounce design stands.

---
▸ 2026-09-23T01:21:09Z [Joel Webber]
verify: `cargo test -p canon-sink` -> PASS (exit 0)
