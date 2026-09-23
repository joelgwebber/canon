---
id: canon-2bb5
title: DLNA GENA eventing to cut state latency
type: task
priority: 3
created: '2026-09-23T16:55:41Z'
updated: '2026-09-23T17:31:42Z'
parent: canon-7718
depends_on:
- canon-685a
labels:
- sink
- network
---

From canon-685a. DLNA state is polled every 500ms (GetTransportInfo + GetPositionInfo). The poll stays regardless (position needs it, and eventing is unreliable across the device zoo). A LastChange subscription could still make pause/play/stop edges near-instant. Do not use rupnp subscribe: it binds its callback listener to the first private IPv4, which may be a tunnel. Serve NOTIFY on the session stream server (already bound to the right LAN interface) and SUBSCRIBE with that callback. Renew before TIMEOUT, and fall back silently if the device refuses.

---
▸ 2026-09-23T17:31:42Z [Joel Webber]
Design note from the gapless/eventing discussion: serve NOTIFY from the per-session HTTP server (today the stream server), which means generalising it from a stream server into a session endpoint (streams + /notify/<sid>) passed into canon_sink::connect. If canon-948b lands upstream, reuse rupnp SUBSCRIBE/renew and propertyset/LastChange parsing rather than writing our own. The Cast analogue is canon-bbc7 (push status via an async client).
