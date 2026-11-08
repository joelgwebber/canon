---
id: canon-d24e
title: Cast thread wedges forever in a blocking status read when the receiver goes quiet
status: shaving
type: bug
priority: 1
created: '2026-11-08T15:35:18Z'
updated: '2026-11-08T15:38:26Z'
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

---
▸ 2026-11-08T15:35:26Z [canon-j15r]

Moved all canon debugging to this thread, on j15r; the old Mac player is killed. Server on j15r: pid 309725, `canon serve --bind 100.75.237.107:7345`, paired players only.

Evidence from the Mac player's log (`~/Library/Application Support/canon/logs/canon.log`, session from 14:27:32 UTC) and a `sample` of pid 39852:

- six tracks on one Cast session, all `consumer gone` with `ended_by_us=true`, no ring lag, no feed stall, no WARN or ERROR
- 15:15:29 Tunes sent an unsolicited `Idle (Finished)` after track 6; every poll after that returned `entries: []`
- 15:20:29.565 last heartbeat line, then silence; the `canon-cast` thread blocked in `get_status` -> `read_tls` -> `recvfrom` in all 872 samples

---
▸ 2026-11-08T15:37:01Z [canon-j15r]

Why the queue went idle in the first place is its own bug: canon-9770 (a Tidal 4005 at 15:15:01 made every remaining track get skipped, player `state=Error`, so nothing was loaded after track 6 and Tunes went Idle at 15:15:29).

That gives a sharper hypothesis for the wedge than "the receiver goes quiet at random": the last poll the player completed was at 15:20:29.565, **exactly five minutes** after Tunes' `Idle (Finished)` at 15:15:29. Cast receivers shut down an idle app (the Default Media Receiver) after about five minutes. After that, `get_status` on the media channel asks a session that no longer exists, and `rust_cast` waits in `receive_find_map` for a reply that never comes (any CLOSE or RECEIVER_STATUS frame it gets meanwhile is just buffered).

Check before building a fix:

- [ ] reproduce on Tunes: load a track, let it finish with nothing queued, and watch for the `get_status` read to stop returning at ~5:00 (trace log, `canon_sink::cast=trace`)
- [ ] if it is the idle shutdown, the cheap fix is to handle it: stop polling the media channel once the receiver goes Idle, and treat the app going away as the session's end (reconnect on the next load) rather than keep polling a dead session; the watchdog is still wanted as the backstop for any other silent receiver

---
▸ 2026-11-08T15:38:26Z [canon-j15r]

Server restarted from this worktree's build (pid 520061, 100.75.237.107:7345, log target/serve.log in the canon thread's worktree, RUST_LOG=info,canon_sink=trace,canon_audio=debug,canon_core::player=debug). No player is connected; the Mac player is dead.
