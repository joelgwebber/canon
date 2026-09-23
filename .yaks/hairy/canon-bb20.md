---
id: canon-bb20
title: macOS discovery via system DNS-SD (mDNSResponder) to bypass Local Network privacy
type: task
priority: 2
created: '2026-09-23T01:21:22Z'
updated: '2026-09-23T03:27:05Z'
parent: canon-f495
labels:
- discovery
- network
- macos
---

On macOS, canon's own multicast sockets (mdns-sd) are silently denied INBOUND LAN traffic by Local Network privacy unless the process/app holds the permission: sockets bind + join + query fine, but no responses arrive. The system resolver (mDNSResponder, via dns-sd/DNSServiceBrowse) is exempt and works. PROPER FIX (use the OS API): a macOS discovery backend over the system DNS-SD API (crate astro-dnssd, or FFI to <dns_sd.h>, or the framework), behind the same DiscoveryEvent/DeviceCache seam ea5d already exposes; keep mdns-sd for Linux/Windows via cfg. Headless Linux has NO such restriction, so this is mainly for macOS desktop/dev. Interim: grant Zed (or Terminal) Local Network permission; a shipped canon.app needs the multicast entitlement + Local Network usage string and a one-time grant. Also: document the macOS Local Network requirement in the README/help.

---
▸ 2026-09-23T01:32:27Z [Joel Webber]
Confirmed workaround: granting the host process (Zed, after update/restart) macOS Local Network permission makes mdns-sd discovery work fully — all 4 devices found. So the system-DNS-SD backend is an ergonomics/robustness improvement for a shipped macOS app (entitlement + usage string + grant), not a blocker for dev or for headless Linux.

---
▸ 2026-09-23T01:54:18Z [Joel Webber]
CORRECTION: this yak's premise (TCC Local Network blocking inbound) was only half right. mDNS discovery WAS TCC-gated (fixed by granting the host process). But inbound TCP to our LAN listener is blocked by the separate macOS APPLICATION FIREWALL keyed on code signature — see canon-dde4's note. Keep this yak scoped to the mDNS/system-DNS-SD question; the firewall/signing issue is packaging and belongs with a 'ship a signed macOS bundle' yak.

---
▸ 2026-09-23T03:15:23Z [Joel Webber]
ASSESSMENT (user asked: is anything left here, and would the native API make it less flaky?).
Short answer: yes it would genuinely help, but it is an ERGONOMICS/ROBUSTNESS win rather than a fix for something broken today, and its headline benefit is gated on canon-742b.
What it would actually buy:
1. PROMPT INSTEAD OF SILENCE. Raw multicast sockets fail silently when Local Network access is denied — exactly what we hit (join+query succeeded, zero inbound, no error, no prompt). Going through mDNSResponder means the OS knows who is asking and can surface the standard Local Network prompt. That is the single biggest flakiness reduction: a user gets a dialog instead of an inexplicably empty device list.
2. DELEGATES THE HARD PARTS. mDNSResponder already handles sleep/wake, interface add/remove/renumber, and cache refresh. That is a large share of what ea5d hand-rolls (resync(), per-NIC membership anchors, the wedge watchdog). Apple's stack does it better and is always running anyway.
3. NO SECOND mDNS STACK on the box; no 5353 coexistence question with mDNSResponder.
Caveats that keep it deferred:
- The prompt/grant keys on code identity, and cargo binaries are ad-hoc/linker-signed with a NEW identity every build (proven by the firewall entries in canon-742b). So the prompt only reliably sticks for a signed bundle => the real payoff is 742b + bb20 together.
- It does NOT fix the other macOS gate: the Application Firewall blocking INBOUND TCP to our LAN stream server is not mDNS at all (canon-742b).
- Deployment target is headless Linux, where none of this applies; mdns-sd is the right backend there regardless, so this is a cfg(target_os="macos") second backend, not a replacement.
Status today: works on the dev machine with the grant in place. Recommend keeping deferred, sequenced after/with canon-742b. Cheap interim option if it bites again: detect the signature (queries sent, zero responses, while the system resolver sees devices) and print a targeted Local Network hint instead of a bare 'no renderers found'.
