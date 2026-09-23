---
id: canon-e920
title: Per-track stream paths on the LAN stream server (gapless groundwork)
type: task
priority: 2
created: '2026-09-23T14:50:48Z'
updated: '2026-09-23T16:34:29Z'
parent: canon-7718
labels:
- arch
- sink
- network
verify: cargo test -p canon-sink --lib
---

From canon-ba30 (E). A network session serves one URL (/stream.flac) and each track installs a fresh FlacTap over one broadcaster, so there is never a second URL to hand a renderer. DLNA SetNextAVTransportURI (and any gapless/preload on Cast queues) needs the next track to be addressable while the current one plays: serve /stream/<generation>.flac, one broadcaster per path, with header replay unchanged. Keep consumers() liveness per session (sum, or the current path). Do this alongside canon-685a if gapless is attempted there.

---
▸ 2026-09-23T16:34:24Z [Joel Webber]
Done. StreamRoutes gives each load its own path (/stream/<n>.flac) and broadcaster, and keeps the current and previous streams addressable. StreamBroadcaster::finish ends every body after what was pushed (an empty chunk is the end marker), and a late subscriber gets the header then an immediate end. It subscribes before checking the flag, so no joiner can miss the end. FlacTap finishes its stream on drop. Unknown or retired paths 404. Session liveness sums consumers across the session streams, and StreamRoutes::is_drained lets the watchdog stand down once the newest stream has been fed in full (the renderer then holds the whole track and stops pulling: 15s+ of buffer on the LS50). This is the groundwork for gapless on DLNA (SetNextAVTransportURI can now point at the next path). Tests: finish_ends_every_subscriber_after_what_was_pushed, a_subscriber_after_finish_gets_the_header_and_an_end, a_session_keeps_the_current_and_previous_streams, a_session_is_drained_once_its_newest_stream_is_fed, session_consumers_sum_across_its_streams, an_unknown_stream_is_404. On-metal evidence is on canon-a70f.

---
▸ 2026-09-23T16:34:27Z [Joel Webber]
verify: `cargo test -p canon-sink --lib` -> PASS (exit 0)
