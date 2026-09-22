---
id: canon-ea5d
title: Resilient discovery supervisor (iface-aware, wake-safe)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T22:25:54Z'
parent: canon-7718
labels:
- discovery
- network
---

A long-lived supervisor per protocol with its own cache and add/remove/expiry events and a debounced snapshot the picker reads (no wholesale replace per scan). Fixes tide-6fd0/8f5b: enumerate real LAN interfaces and EXCLUDE tunnels (utun/tun/tap/VPN) — a stale utun 224.0.0/4 route silently black-holed tideway's multicast; pin IP_MULTICAST_IF per chosen iface; rebuild the socket + re-add IP_ADD_MEMBERSHIP on sleep/wake; watchdog that re-bursts M-SEARCH when reception looks wedged. Crates: socket2 (multicast control), mdns-sd (Cast browser), ssdp-client/rupnp (SSDP+descriptors).
