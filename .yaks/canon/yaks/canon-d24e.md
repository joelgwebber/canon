---
id: canon-d24e
title: Cast thread wedges forever in a blocking status read when the receiver goes quiet
status: shaving
type: bug
priority: 1
created: '2026-11-08T15:35:18Z'
updated: '2026-11-08T15:35:26Z'
parent: canon-5303
tags:
- cast
- sink
- network
---

Found 2026-11-08 on a real `canon player` (macOS, driven by a j15r server over the tailnet), streaming to Tunes@cast. After 53 minutes and six clean tracks the player stopped doing anything: no more logs, playback commands ignored, the Cast and stream TCP connections still ESTABLISHED with empty queues.

`sample` on the player showed the `canon-cast` thread in `run_connection` -> `MediaChannel::get_status` -> `receive_find_map` -> rustls `read_tls` -> `recvfrom`, in all samples: a blocking read for a status reply Tunes never sent. Tunes still answered ping and accepted new connections on 8009; only the existing session went quiet.

That one thread owns all Cast I/O and is the only one that drains the command queue (see the module docs in `crates/canon-sink/src/cast.rs`), so once it blocks, play/pause/seek/load/sink changes all do nothing, and nothing is logged or reported to clients. A seek, skip or sink change cannot get through either: the command queue is only drained between reads. `rust_cast` has no read timeout and does not expose its socket.

Fix direction: a read has to be bounded. Options, in the order to try: (1) a watchdog that notices a poll/heartbeat has not completed within a deadline and shuts down the socket (or otherwise forces the read to fail), so the existing "renderer connection dropped; reconnecting" path in the controller takes over; (2) own the TLS socket and set a read timeout, which means patching or replacing `rust_cast` (canon-bbc7). Either way, say so to clients (canon-60fa).

Reproduce with `scripts/netfault.c` against Tunes (`touch target/netfault/drop-8009` drops the next read on a connection to that port): the fault is the *receiver* going quiet, so the fake has to be on the read side.

Possibly the same cause as the original "audio cuts out mid-stream, play head keeps moving, seek/skip fixes it" report, but nothing logged explains that one, so it may be a second bug.

Also open: at 15:15 Tunes reported Idle/Finished after track 6 and no next track was loaded; check the server log to see whether the queue had ended or the load was never issued.

---
▸ 2026-11-08T15:35:26Z [canon-j15r]
kind: move
from: hairy
to: shaving
thread: ksQQ3YaazyXkR9WZ-OQ-DusyzJLOAE20MJIBxBCA0aLJyrhJPreUvVwLftHG
---

moved: hairy -> shaving
