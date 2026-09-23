---
id: canon-ea5d
title: Resilient discovery supervisor (iface-aware, wake-safe)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-23T02:34:38Z'
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

---
▸ 2026-09-23T01:32:27Z [Joel Webber]
ON-METAL VALIDATION PASSED (after granting macOS Local Network permission to the host process): 'canon devices' found all 4 LAN renderers with friendly names — Basement speaker 192.168.0.30, Kitchen 192.168.0.7, Library display 192.168.0.82, Tunes (KEF LS50 Wireless II) 192.168.0.205, all :8009. Confirms end-to-end: usable_interfaces chose ONLY en0 out of ~25 host interfaces (excluding 6 utun tunnels, awdl0, llw0, bridge0, anpi*, lo0), per-NIC pinning + group join work, fn= TXT -> friendly name, and the debounced cache publishes a stable sorted snapshot. The earlier empty result was purely the macOS privacy block (canon-bb20).

---
▸ 2026-09-23T02:24:47Z [Joel Webber]
REOPENED (2nd) — real bug found only by a LONG-RUNNING daemon: the device list starts correct, then goes EMPTY after ~2 minutes and never recovers, while a fresh 'canon devices' process still finds all 4.
ROOT CAUSE: my DEFAULT_TTL (120s) expiry reaps entries based on when CANON last saw a ServiceResolved event. But mdns-sd maintains its own record cache and re-queries on its own schedule (refresh_active_services / CacheRefreshPTR), and only re-emits ServiceResolved on a CHANGE — a still-present device that its cache refreshed silently produces no new event. So my TTL deletes perfectly live devices and nothing ever re-adds them.
This is my bug, not mdns-sd's: it already emits ServiceRemoved when its own records expire (service_daemon.rs:3081/3749), so it IS the authoritative liveness signal. Layering a second, blind TTL on top second-guesses the layer that actually knows. FIX: drop the redundant TTL and let the source's add/remove events drive the cache (keeping TTL support in the cache for a future protocol, e.g. SSDP, whose events genuinely need it).

---
▸ 2026-09-23T02:34:38Z [Joel Webber]
verify: `cargo test -p canon-sink` -> PASS (exit 0)

---
▸ 2026-09-23T02:34:38Z [Joel Webber]
FIXED + verified on-metal. Dropped the redundant blind TTL: DEFAULT_TTL is now None for the mDNS source, so mdns-sd's own add/remove events (it re-queries on its own schedule and emits ServiceRemoved when its records genuinely expire) are the authoritative liveness signal. DeviceCache/SnapshotEngine keep OPTIONAL ttl support because a polled protocol (SSDP, whose M-SEARCH responses carry CACHE-CONTROL lifetimes) will need it.
Evidence:
- New regression test discovery::tests::without_a_ttl_devices_never_expire: a device stays listed 6 hours later with no further events, and an explicit source Removed still works. 27 canon-sink tests pass, clippy 0 warnings, fmt clean.
- LIVE: long-running daemon polled over the ws every 65s — 5 sinks (Local + Basement speaker + Kitchen + Library display + Tunes) at t+0s, t+65s, t+130s, t+195s. Previously the list emptied at ~120s and never recovered.
