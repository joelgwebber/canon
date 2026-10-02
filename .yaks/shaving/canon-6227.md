---
id: canon-6227
title: No DLNA devices appear on Linux
type: bug
priority: 1
created: '2026-10-01T11:30:04Z'
updated: '2026-10-02T03:28:05Z'
labels:
- linux
- network
- dlna
---

But they do on the same network from MacOS. The local Tunes KEF speakers expose both.

---
▸ 2026-10-01T11:33:43Z [Joel Webber]
Diagnosis 2026-10-01: ufw is active here with DEFAULT_INPUT_POLICY=DROP and no user rules. Its before.rules accept only multicast mDNS (224.0.0.251:5353) and SSDP NOTIFY (239.255.255.250:1900). canon's SSDP source sends M-SEARCH from an ephemeral port, and the replies come back as unicast UDP from :1900 to that port; ufw drops them all (journalctl -k: '[UFW BLOCK] SRC=192.168.0.205 DST=192.168.0.27 PROTO=UDP SPT=1900 DPT=<ephemeral>', dozens across a few minutes). Cast discovery survives only because mDNS is multicast. The same policy blocks the renderer's inbound TCP fetch from the LAN stream server (random port), so Cast and DLNA playback both fail even once found (see canon-775e). macOS has no such default-deny for these, hence it works there.

---
▸ 2026-10-01T11:33:43Z [Joel Webber]
Fix shape? (a) Joel opens the LAN in ufw (sudo ufw allow from 192.168.0.0/24), no code; (b) canon gets fixed, configurable ports for the stream server and the SSDP search socket, plus a documented narrow ufw rule (or an /etc/ufw/applications.d profile), and a clearer hint in the 'never fetched' error on Linux; optionally (c) also listen for SSDP NOTIFY on 239.255.255.250:1900, which default ufw already admits, so devices appear (slowly) without M-SEARCH replies.

---
▸ 2026-10-02T03:21:20Z [Joel Webber]
Joel (2026-10-01): (b) with detection: fixed, configurable ports for the stream server and the SSDP search socket; detect ufw and say exactly what to allow; document it; ship a ufw app profile. Make sure it works once the rule is in place. Fixed SSDP source port is fine by the protocol (replies go to the query's source ip:port; avoid 1900 itself); the catch is one owner per port, so a busy port falls back to ephemeral for that search. The stream server becomes one long-lived server with per-session path prefixes, since sessions overlap on a speaker switch.

---
▸ 2026-10-02T03:28:05Z [Joel Webber]
Built 2026-10-01: one --lan-port (CANON_LAN_PORT, default 7346) for both the stream server (TCP) and DLNA M-SEARCH source (UDP), so one ufw rule admits both. Stream server is now one long-lived StreamServer (canon-sink) with per-session prefixes /<session>/stream/<n>.flac, started on first network session (restarted if the LAN IP changes); NetworkSession holds a Registration whose Drop unregisters and finishes its streams. A taken port (second canon) falls back to OS-picked with a warning (stream) / debug (ssdp). canon serve on Linux reads /etc/ufw/{ufw.conf,user.rules} + /etc/default/ufw and warns with the exact 'sudo ufw allow from <lan> to any port 7346' (firewall.rs, 5 unit tests over real ufw rule shapes: port rules, app-profile multiport, network-wide, ranges, DROP). packaging/ufw/canon profile, docs/firewall.md, README/arch/AGENTS updated.

---
▸ 2026-10-02T03:28:05Z [Joel Webber]
Live pre-rule 2026-10-01 (Linux, :7399, no ufw rule yet): startup 'WARN canon::firewall: ufw is on and drops port 7346: speakers can't fetch canon's stream, and DLNA devices won't be found. Allow it once with: sudo ufw allow from 192.168.0.0/24 to any port 7346'. sink Tunes@cast -> 'stream server on http://192.168.0.27:7346', loss names http://192.168.0.27:7346/1. journalctl -k from Tunes: 1x PROTO=TCP DPT=7346, 4x PROTO=UDP SPT=1900 DPT=7346: everything canon needs now lands on the one port. OWED: re-run with Joel's rule in place (DLNA Tunes listed; cast + dlna play, seek, pause) before shearing.
