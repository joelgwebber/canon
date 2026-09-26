---
id: canon-5305
title: 'Seek within Spotify tracks: let a seekable source ask the engine to start mid-track'
type: task
priority: 2
created: '2026-09-26T21:18:57Z'
updated: '2026-09-26T21:22:47Z'
parent: canon-c6ac
labels:
- audio
- spotify
verify: cargo test -p canon-audio a_seekable && cargo build -p canon-daemon
---

A seek on a Spotify track restarts it at 0 (start_ms 0, honest but useless). librespot's decrypted file is seekable; canon's engine presents every input as forward-only. A ResolvedStream can say its bytes are seekable and where to start; the engine then decodes from that time (Symphonia seek) and reports where it really landed, so the clock stays true. Also the first step for seekable local files (canon-6a92).

---
▸ 2026-09-26T21:22:45Z [Joel Webber]
Built: ResolvedStream.seek_to (a source with seekable bytes asks the engine to decode from a point); AudioPlayer::start/run/run_network take seek; Decode::open presents the input as seekable (length measured up front) when asked, seeks the demuxer (SeekMode::Accurate, SeekTo::Time), resets the decoder and reports started_at_ms (the packet boundary it landed on), which becomes the stream start for Loaded and the clock. SpotifyAudio sets seek_to from the requested position. Unit test on tests/assets/aac_plain.m4a: asked 1200 ms, landed within 1000..=1200 (that asset has its moov at the end and cannot be opened forward-only at all: the canon-6a92 case, which a seekable input fixes).

---
▸ 2026-09-26T21:22:45Z [Joel Webber]
Live 2026-09-26, test daemon :7399, Era (Spotify-only, from spotify:7isRVNddDK5D9OcwhuFQoJ, vorbis 44.1) at volume 0: playing 0:04 -> seek 3:00 -> 3:04 after 5 s; seek +30 -> 3:38; seek 1:00 (backward) -> 1:03; every restart still from Spotify, no errors.

---
▸ 2026-09-26T21:22:47Z [Joel Webber]
verify: `cargo test -p canon-audio a_seekable && cargo build -p canon-daemon` -> PASS (exit 0)
