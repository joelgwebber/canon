---
id: canon-9bce
title: 'Cast connect has no timeout: a speaker that ignores TCP hangs sink selection for minutes'
type: bug
priority: 2
created: '2026-10-02T03:39:44Z'
updated: '2026-10-02T03:39:44Z'
labels:
- cast
- sink
- network
---

Found 2026-10-01 on Linux while verifying canon-6227. Kitchen (Nest Hub, 192.168.0.7) answered ping but silently dropped TCP to :8009 (a bash /dev/tcp probe timed out after 4s; Basement and Tunes connected at once). 'sink Kitchen' then got no reply and no log line: rust_cast's CastDevice::connect_without_host_verification does a blocking TcpStream::connect with the OS default timeout (~2 min on Linux), and connect_settled retries it CONNECT_ATTEMPTS times. The client sees nothing; playback stays where it was. Bound the connect (connect with a timeout ourselves and hand rust_cast the stream, or run it under a deadline and report 'Kitchen isn't answering on 8009'), and say so in the reply. Not caused by canon-6227's port change: the stream server had already started on :7346 when it hung.
