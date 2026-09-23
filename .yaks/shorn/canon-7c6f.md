---
id: canon-7c6f
title: Device identity separate from protocol endpoint (one speaker, many protocols)
type: task
priority: 2
created: '2026-09-23T14:50:48Z'
updated: '2026-09-23T16:56:00Z'
parent: canon-7718
labels:
- arch
- sink
verify: cargo test -p canon-sink --lib renderer
---

From canon-ba30 (F). SinkId is the mDNS service name, so a speaker that speaks Cast and DLNA (Tunes) will appear with two unrelated ids once canon-685a lands. Introduce a device identity (UPnP UDN / Cast id / IP as fallback) that groups endpoints, and decide how list_sinks presents it. Proposed: one entry per physical device with a preferred protocol, plus the available protocols listed so an A/B remains possible (for example sink Tunes@dlna). This answers the list_sinks question recorded on canon-685a.

---
▸ 2026-09-23T16:55:56Z [Joel Webber]
Done. Device identity = host address. Measured on the LS50: Cast id d8bf502c... and UPnP UDN c353baec-... are unrelated, so the address is the only shared key. canon_sink::outputs groups discovery into one SinkInfo per physical output: id/kind are the preferred protocol (Cast, then DLNA), and protocols lists every endpoint, so it is backward compatible (selecting id is the right default). canon control: `sink Tunes` takes the preferred protocol, `sink Tunes@dlna` / `@cast` pins one, and `sinks` shows `(also dlna)`. Switching protocols on the same speaker releases it before connecting (a Cast launch while DLNA plays is refused with CANCELLED and knocks DLNA over), connects with up to 3 tries 1s apart, and falls back to local if that still fails.

On-metal evidence:
  sinks: chromecast Tunes LS50-Wireless-II-...  (also dlna)   one entry, not two
  Before the release-first fix, Cast->DLNA->Cast: ERROR sink: launch media receiver: ... Could not run application (CANCELLED), then the DLNA session watchdog failed back to local 10s later.
  After, sink Tunes / enqueue / sink Tunes@dlna / sink Tunes / sink Tunes@dlna / stop:
  12:53:57 playing LS50 (cast) -> 12:54:11 playing dlna -> 12:54:28 playing LS50 (cast) -> 12:54:41 playing dlna -> 12:54:51 idle
  No errors, no warnings, no connect retries needed.

---
▸ 2026-09-23T16:56:00Z [Joel Webber]
verify: `cargo test -p canon-sink --lib renderer` -> PASS (exit 0)
