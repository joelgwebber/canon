---
id: canon-6227
title: No DLNA devices appear on Linux
type: bug
priority: 1
created: '2026-10-01T11:30:04Z'
updated: '2026-10-01T11:33:52Z'
labels:
- linux
- network
- dlna
needs: human
---

But they do on the same network from MacOS. The local Tunes KEF speakers expose both.

---
▸ 2026-10-01T11:33:43Z [Joel Webber]
Diagnosis 2026-10-01: ufw is active here with DEFAULT_INPUT_POLICY=DROP and no user rules. Its before.rules accept only multicast mDNS (224.0.0.251:5353) and SSDP NOTIFY (239.255.255.250:1900). canon's SSDP source sends M-SEARCH from an ephemeral port, and the replies come back as unicast UDP from :1900 to that port; ufw drops them all (journalctl -k: '[UFW BLOCK] SRC=192.168.0.205 DST=192.168.0.27 PROTO=UDP SPT=1900 DPT=<ephemeral>', dozens across a few minutes). Cast discovery survives only because mDNS is multicast. The same policy blocks the renderer's inbound TCP fetch from the LAN stream server (random port), so Cast and DLNA playback both fail even once found (see canon-775e). macOS has no such default-deny for these, hence it works there.

---
▸ 2026-10-01T11:33:43Z [Joel Webber]
Fix shape? (a) Joel opens the LAN in ufw (sudo ufw allow from 192.168.0.0/24), no code; (b) canon gets fixed, configurable ports for the stream server and the SSDP search socket, plus a documented narrow ufw rule (or an /etc/ufw/applications.d profile), and a clearer hint in the 'never fetched' error on Linux; optionally (c) also listen for SSDP NOTIFY on 239.255.255.250:1900, which default ufw already admits, so devices appear (slowly) without M-SEARCH replies.
