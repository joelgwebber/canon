---
id: canon-bb20
title: macOS discovery via system DNS-SD (mDNSResponder) to bypass Local Network privacy
type: task
priority: 2
created: '2026-09-23T01:21:22Z'
updated: '2026-09-23T01:32:27Z'
parent: canon-7718
labels:
- discovery,network
---

On macOS, canon's own multicast sockets (mdns-sd) are silently denied INBOUND LAN traffic by Local Network privacy unless the process/app holds the permission: sockets bind + join + query fine, but no responses arrive. The system resolver (mDNSResponder, via dns-sd/DNSServiceBrowse) is exempt and works. PROPER FIX (use the OS API): a macOS discovery backend over the system DNS-SD API (crate astro-dnssd, or FFI to <dns_sd.h>, or the framework), behind the same DiscoveryEvent/DeviceCache seam ea5d already exposes; keep mdns-sd for Linux/Windows via cfg. Headless Linux has NO such restriction, so this is mainly for macOS desktop/dev. Interim: grant Zed (or Terminal) Local Network permission; a shipped canon.app needs the multicast entitlement + Local Network usage string and a one-time grant. Also: document the macOS Local Network requirement in the README/help.

---
▸ 2026-09-23T01:32:27Z [Joel Webber]
Confirmed workaround: granting the host process (Zed, after update/restart) macOS Local Network permission makes mdns-sd discovery work fully — all 4 devices found. So the system-DNS-SD backend is an ergonomics/robustness improvement for a shipped macOS app (entitlement + usage string + grant), not a blocker for dev or for headless Linux.
