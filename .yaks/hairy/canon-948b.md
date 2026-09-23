---
id: canon-948b
title: 'Upstream: interface-pinned SSDP search and caller-supplied GENA callback in rupnp/ssdp-client'
type: chore
priority: 4
created: '2026-09-23T17:31:04Z'
updated: '2026-09-23T17:31:04Z'
labels:
- upstream
- sink
- network
---

Low priority, good-citizenship work. Not blocking anything.

PROBLEM (read from source, 2026-09-23):
- ssdp-client 2.1 search(): on Unix it binds 0.0.0.0:0 and never sets IP_MULTICAST_IF, so the route table chooses the egress. On Windows it "connects" to 8.8.8.8 and binds whatever local address that picks. After a VPN has come and gone, a stale 224.0.0.0/4 route on a dead utun wins, and M-SEARCH goes nowhere, silently: the tideway black hole.
- rupnp 3.0 discover()/discover_with_properties() call that search with no way to choose an interface.
- rupnp subscribe() binds its NOTIFY listener to utils::get_local_addr(), "the first private IPv4 of if-addrs", which may be a tunnel. The subscribe stream parser is also line-based (it assumes "<e:propertyset" at the start of a line), and the NOTIFY body is not exposed as a parse function, so a caller cannot serve NOTIFY from its own HTTP server.
- Minor: DeviceSpec::udn() is behind the full_device_spec feature, although every device has a UDN.

WHAT UPSTREAM WOULD NEED:
1. ssdp-client: search_from(local: Ipv4Addr, ...) (bind to it, set IP_MULTICAST_IF, TTL), or accept a caller-built UdpSocket.
2. rupnp: pass-throughs for that (discover_from), subscribe_with_callback(url, callback: &str, timeout) that only sends SUBSCRIBE, plus a public parse_propertyset(body) -> HashMap and a LastChange (AVTransport event XML) parser.
3. Optionally make udn() unconditional.

WHAT IT WOULD SIMPLIFY IN CANON:
- canon-sink::ssdp (~180 lines with tests) would reduce to a call to rupnp::discover_from per usable interface. That is modest: ours is small and well tested.
- canon-2bb5 (DLNA eventing) could reuse upstream SUBSCRIBE/renew and propertyset/LastChange parsing, served from our own session HTTP server, instead of writing NOTIFY and LastChange parsing ourselves. This is the more valuable half.
If upstream is slow or unresponsive, nothing is lost: we already work around all of it.
