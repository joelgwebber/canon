---
id: canon-040e
title: Server FLAC stream endpoint for players
status: shorn
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-18T04:18:48Z'
parent: canon-88a7
source: arch
---

The server resolves a track through `Sources` (Tidal fMP4/FLAC, librespot Ogg Vorbis, local files), decodes it, and serves FLAC at the source's native bit depth and rate over plain HTTP, starting at a requested position. Seek = a new request from the position (renderer-style); no byte ranges yet. The URL is handed to the executor inside the start/prepare effect, carries a short-lived signed token (see the auth yak), and is what a native player would play directly.

The 16-bit `FLAC_BITS` cap stays a renderer rule on the player, not here.

---
▸ 2026-10-17T20:37:28Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T20:40:26Z [worker-040e]

Start: Sat Oct 17 08:40:26 PM UTC 2026; yaks path: /home/joel/src/canon/.delta/worktrees/8h7pjanex4j2/canon/.yaks/canon-040e.md; worktree /home/joel/src/canon/.delta/worktrees/8h7pjanex4j2/canon

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: the endpoint is a route in canon-api (`GET /audio/{id}`, crates/canon-api/src/audio.rs) over a trait `canon_api::AudioSource` that the daemon implements (`ServerAudio`, crates/canon-daemon/src/audio.rs). Rejected: a router the daemon builds and merges onto the API router. The daemon has no axum dependency, so naming `Router` and serving the merge there needed a Cargo.toml edit, and it would leave /ws and /audio with two places to put the auth check. With the trait, canon-api owns routes, headers, status codes and the single `authorize` function; canon-api still links neither canon-audio nor canon-sink. The trait is hand-boxed (`BoxFuture`) because async-trait is only a dev-dependency of canon-api. No Cargo.toml or Cargo.lock edits.

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: URL is `GET /audio/<track entity id>?position_ms=<ms>` (id parsed by serde as EntityId; bad id -> 400, unknown -> 404, no AudioSource wired -> 501). The response is `audio/flac`, chunked, with header `x-canon-start-ms` = where the first sample sits in track time. Rejected: a `.flac` suffix on the path (axum param-with-suffix gains nothing), and putting the start in a FLAC metadata block (players would have to parse it; a header is readable before the body and by a bare-URL native player's HTTP layer).

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: the start reported is the source's own: `PcmStream::start_ms` = the demuxer's landing point after a `seek_to`, else `ResolvedStream::start_ms` (same rule as the engine's run_network). The stream is NOT trimmed to the requested position. Rejected: dropping the leading (position - start) frames so the stream starts exactly where asked. It would hide a Tidal segment-start offset from the player, but the brief asks the response to report the real start, renderers already work the same way, and trimming can be added later without a wire change (the header would just equal the request). Follow-up, not done.

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: bit depth. FLAC/ALAC sources are served at the codec's declared depth (capped at 24); anything with no integer samples (AAC, Vorbis, MP3) at 16 (`LOSSY_BIT_DEPTH`). Samples are recovered exactly: Symphonia widens an N-bit sample to f32 by /2^(N-1), a power of two, so up to 24 bits the f32 round trip is lossless and `PcmStream` multiplies back by 2^(N-1). Test proves 24-bit/96 kHz stereo is bit-perfect, whole and after a seek. Rejected: reusing `FlacTap`, whose f32->i32 scale is 2^(N-1)-1, so it is off by up to 1 LSB at peak against what Symphonia produces (existing renderer path, untouched; see surprising notes).

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: encoder. Extracted `FlacEncoder` (i32 in, FLAC frames out, no broadcaster, no pacing) in canon-sink/src/flac_encode.rs; `FlacTap` is now a thin wrapper (f32->i32 + broadcaster) around it, behaviour unchanged (its 9 tests pass untouched). Rejected: feeding `FlacTap`+`StreamBroadcaster` per request. The broadcaster is a live-edge ring that disconnects a lagging reader, the wrong model for a player pulling faster than realtime; here HTTP backpressure is the pacing.

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: decode path. `Decode` in canon-audio/src/engine.rs is now pub(crate) and records `bits_per_sample`; new `canon_audio::PcmStream` (src/pcm.rs) wraps it. Rejected: a second Symphonia decode loop (duplicates probe, seek and landing logic, which has been paid for on hardware). One engine behaviour change: `Decode::open` now treats a source that offers any seek (even to 0) as seekable (`ForwardSource` byte length measured). Before, a seek_to of 0 left the input forward-only, which can't open an MP4 with its index at the end; found by the aac_plain.m4a test. Engine tests still pass.

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: streaming mechanics. Opening is awaited before headers go out (the start and format must be known), then a dedicated std thread (as the engine's feed has; decode blocks on source reads) pumps FLAC frames into a bounded mpsc (8 chunks) that is the response body. A slow player blocks the thread, a gone player (receiver dropped) ends it. A decode/encode failure mid-stream sends an io::Error so hyper aborts the connection instead of ending the body cleanly, which would look like end-of-track. Rejected: spawn_blocking (long-lived jobs would eat the blocking pool), and pacing to realtime (players want to buffer ahead).

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: quality. The endpoint opens sources at `Quality::HiRes` (main.rs), not the `Lossless` the controller uses, because the brief says native depth and rate and sources clamp to what the account can give. Rejected: Lossless (would cap Tidal at 16/44.1, defeating hi-res). Not verified against Tidal (no credentials): that a hi-res Tidal stream decodes through this path is assumed from the fMP4/FLAC spike (canon-c4c3) and the 24-bit/96k round trip test.

---
▸ 2026-10-17T20:51:36Z [worker-040e]

Decision: auth seam is one function, `authorize(state, track, token: Option<&str>) -> Result<(), StatusCode>` in canon-api/src/audio.rs, called first in the handler; the request's `?token=` is parsed into it. It passes everything today. canon-65d2 replaces its body. Rejected: a trait object on AppState (more surface than the single call needs, and conflicts with the auth module landing separately).

---
▸ 2026-10-17T20:51:53Z [worker-040e]

Evidence, green bar (cat target/green-040e.txt):
cargo fmt --all --check            -> FMT_OK
cargo clippy --workspace --all-targets -> "Finished `dev` profile ... in 0.77s", zero warnings (an earlier run flagged clippy::should_implement_trait on PcmStream::next; renamed next_chunk)
cargo test --workspace             -> exit 0, passed 403 failed 0 (sum of every "test result" line)
cargo build --workspace            -> Finished `dev` profile
New tests (crates/canon-daemon/src/audio.rs, real canon_api::serve on 127.0.0.1:0, real Library, raw HTTP/1.0 client), all ok:
  serves_fragmented_flac_sample_for_sample   (flac_frag.mp4: 44100 Hz, 16 bit, mono, start 0, samples identical to a direct decode)
  reports_where_a_segmented_source_really_starts (position_ms=1500 -> x-canon-start-ms 1000, untrimmed)
  seeks_into_a_seekable_source               (aac_plain.m4a, position_ms=1000 -> start in 900..=1000, 16 bit, length matches the tail from that start)
  serves_hi_res_at_its_native_format         (generated 24-bit/96 kHz stereo FLAC: bit-perfect whole; after position_ms=500 the body is the exact tail of the track and the reported start matches where it lands, +-1 ms)
  refuses_what_it_cannot_serve               (unknown id 404, malformed id 400, no AudioSource 501)

---
▸ 2026-10-17T20:51:53Z [worker-040e]

Evidence, external decoders on the served bytes (the hi-res test, temporarily dumping the response bodies, since removed):
ffprobe -show_entries stream=codec_name,sample_rate,channels,bits_per_raw_sample,duration target/dump-whole.flac -> flac,96000,2,N/A,24   (duration N/A: STREAMINFO total_samples is 0 = unknown, as for the renderer stream)
flac -t target/dump-whole.flac -> "WARNING, cannot check MD5 signature since it was unset in the STREAMINFO" then "ok"
ffprobe / flac -t on target/dump-seek.flac (the position_ms=500 response) -> flac,96000,2,N/A,24 ; ok

---
▸ 2026-10-17T20:51:53Z [worker-040e]

Evidence, live daemon (not on 7345; --bind 127.0.0.1:7398 --lan-port 7348 --state-dir target/st-040e, a `sqlite3 .backup` copy of the library, no credentials; stopped by `pgrep -f 'bind 127.0.0.1:739[8]' | xargs kill`; Joel's 7345 daemon still listening afterwards; the copy migrated 9 -> 10 in the copy only):
curl /audio/13d47c34-8999-4ee2-9eeb-271841ffadcb (a Spotify-bound track) -> 502 "source: tidal: tidal can't stream: sign in with the browser login (tidal.pkce); spotify: spotify can't stream: ..."   (same with ?position_ms=30000)
curl /audio/00000000-0000-4000-8000-000000000000 -> 404 "not found: track 0000..."
curl /audio/nope -> 400 "Invalid URL: Cannot parse `id` with value `nope`: UUID parsing failed ..."
So routing, id parsing, library lookup and error mapping are live-checked; audio flowing from a real source through the running daemon is NOT (see the ask).

---
▸ 2026-10-17T20:51:53Z [worker-040e]

asked: needs human

Remaining live check, left for Joel (needs credentials, or a local-file source the daemon doesn't have; canon serve registers only the Tidal/Spotify/librespot connectors): on a test daemon (--bind 127.0.0.1:7398 --lan-port 7348 --state-dir <own dir holding a copy of library.sqlite AND the real credentials, so not copied by me), pick a Tidal-bound track entity id (sqlite3 library.sqlite "select entity,key from bindings where kind='track' and service='tidal' limit 3"), then: curl -s -D target/h.txt 'http://127.0.0.1:7398/audio/<id>?position_ms=60000' -o target/t.flac; cat target/h.txt (want 200, audio/flac, x-canon-start-ms <= 60000, within a Tidal segment of it); ffprobe target/t.flac (sample rate and bit depth equal Tidal's: 44100/16 or hi-res 24-bit); flac -t target/t.flac; play it (canon play-file target/t.flac) and listen near the cut. Also repeat for a librespot-bound track (Vorbis: want 16-bit and a start near 60000 from seek_to). Also try killing curl mid-download and check the daemon thread exits (no stuck canon-audio-stream thread; ps -L).

---
▸ 2026-10-17T20:51:53Z [worker-040e]

Summary: server FLAC endpoint done and tested; yak left Shaving (not shorn) because the end-to-end live check with a real source needs credentials (asked above). Code: canon-api `audio.rs` (route, `AudioSource` trait, single `authorize` seam, `x-canon-start-ms`), canon-daemon `audio.rs` (`ServerAudio`, wired in main with Quality::HiRes), canon-audio `PcmStream`, canon-sink `FlacEncoder` (FlacTap now wraps it), docs/arch.md section 10. No Cargo.toml/Cargo.lock edits. Surprising: (1) FlacTap's f32->i32 scale is 2^(N-1)-1 while Symphonia widens by 2^(N-1), so the renderer path is up to 1 LSB off at peak; not touched. (2) daemon has no local-file Source, so a local file can't be played through `canon serve`. (3) a seek_to of 0 left plain MP4s unopenable (fixed in Decode::open). (4) `yaks doctor --strict` currently fails on canon-4185 (shorn, no note), not ours.

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

Landed on main (c4293ce); live check run by the agent with Joel's go-ahead.

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

Live, on Joel's real state dir after landing (test daemon 127.0.0.1:7399 / lan 7347), `target/hw-040e.sh`:
- Tidal 33348478, `curl /audio/<id>?position_ms=60000` → `HTTP/1.1 200 OK`, `content-type: audio/flac`, `x-canon-start-ms: 59907` (the segment start, ≤ 60000). Server log: `streaming "Army of Me" from tidal:33348478 (flac 16/44.1 kHz) at 60s`.
- `ffprobe` → `flac,44100,2,16`; `flac -t` → `ok` (MD5 unset, expected for a live stream); decoded 7692180 frames = 174.43s = 3:54 − 59.9s, i.e. the whole remainder of the track.
- Spotify-only track (librespot path): 403 `spotify can't stream: sign in with the librespot login` — the server has no librespot login, so that path is untested live; it's covered by the seekable-source tests. Filed as a follow-up note rather than blocking.
- Also exercised through canon-1ad8's session: a remote player fetched and played Army of Me from 47.788s.

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

moved: shaving -> shorn
