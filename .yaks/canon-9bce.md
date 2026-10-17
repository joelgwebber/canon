---
id: canon-9bce
title: 'Cast connect has no timeout: a speaker that ignores TCP hangs sink selection for minutes'
status: shorn
type: bug
priority: 2
created: '2026-10-02T03:39:44Z'
updated: '2026-10-04T16:50:59Z'
parent: canon-5303
labels:
- cast
- sink
- network
verify: cargo test -p canon-daemon never_answers
---

Found 2026-10-01 on Linux while verifying canon-6227. Kitchen (Nest Hub, 192.168.0.7) answered ping but silently dropped TCP to :8009 (a bash /dev/tcp probe timed out after 4s; Basement and Tunes connected at once). 'sink Kitchen' then got no reply and no log line: rust_cast's CastDevice::connect_without_host_verification does a blocking TcpStream::connect with the OS default timeout (~2 min on Linux), and connect_settled retries it CONNECT_ATTEMPTS times. The client sees nothing; playback stays where it was. Bound the connect (connect with a timeout ourselves and hand rust_cast the stream, or run it under a deadline and report 'Kitchen isn't answering on 8009'), and say so in the reply. Not caused by canon-6227's port change: the stream server had already started on :7346 when it hung.

---
▸ 2026-10-04T16:50:59Z [Joel Webber]

verify: `cargo test -p canon-daemon never_answers` -> PASS (exit 0)

---
▸ 2026-10-04T16:50:59Z [Joel Webber]

Shorn 2026-10-04. connect_settled bounds each attempt at CONNECT_TIMEOUT (8s), and doesn't retry after a timeout (a speaker that ignores us isn't settling); the error names the speaker. The Cast thread checks whether anyone still waits before launching the media app, and disconnects if the caller gave up, so a late connect doesn't take a speaker for nobody. canon control's 'sink' now awaits the select_sink reply, so the failure is printed (it used to fire and forget, which hid every selection error). Unit test: a local listener that accepts TCP and never answers TLS is given up on after the timeout, once. On hardware (test daemon, scripts/netfault.c 'ignore-8009' making connects to Library display hang like Kitchen's did): 'sink Library' printed 'error: sink: Library display (192.168.0.82:8009) didn't answer within 8s' 8s after selecting.
