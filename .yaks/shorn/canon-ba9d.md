---
id: canon-ba9d
title: 'Source seam that the controller actually uses: open-at-position, registry, resolver'
type: task
priority: 2
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T20:55:02Z'
parent: canon-4185
labels:
- arch
- library
verify: cargo test -p canon-core source && cargo build --workspace
---

From canon-ba30 (H). The controller holds Arc<TidalSession>, calls open_stream_at directly, picks tidal_id() off TrackRef, and fails any non-Tidal track. Source::resolve is a dead buffered fallback (fetch_all pulls the whole track into a Cursor) with no start position, which is why the controller could not use it. Change: Source::open(source, quality, start) -> (ResolvedStream, start_ms), a HashMap<Service, Arc<dyn Source>> registry in the daemon, and a resolver that walks TrackRef.sources by policy (local before streaming, per id.rs). Delete the buffered fallback. Also move codec_hint next to StreamInfo (duplicated 3x today). This is the seam canon-880e and canon-5cb2 plug into, and it is small now.

---
▸ 2026-09-23T20:54:52Z [Joel Webber]
Source::open(binding, quality, start) -> ResolvedStream{input, info, start_ms}; Sources registry in canon-core (HashMap<Service, Arc<dyn Source>>, candidates() = local first, else track order; single failure passes through with its kind, several summarised). TidalSource(Arc<TidalSession>) in canon-tidal; buffered fetch_all fallback and open_stream deleted; track_meta is inherent on the session. Controller holds Sources, no TidalSession/tidal_id. codec_hint -> Codec::extension_hint. Registry lives in core (not the daemon) so catalog browsing (canon-b989) and the library can use it too.

---
▸ 2026-09-23T20:54:52Z [Joel Webber]
On metal (2026-09-23), Tunes Cast flow: enqueue 55391795 55391796, seek 3:30 -> loading, then playing from 207679 (segment start, start_ms honoured), Brain Damage clamped at 230000 then (12, playing, index 1, Eclipse) pos 65->5429 (prepare via Sources, flow join). Local: seek 3:35 -> playing from 211672, 230000 then (9, playing, 1, Eclipse) pos 57->6059 (gapless). Found canon-3a6c (watchdog fails an idle output back to local) on the first run, when enqueue took both ids as one bogus id; enqueue now takes several ids.

---
▸ 2026-09-23T20:54:55Z [Joel Webber]
verify: `cargo test -p canon-core source && cargo build --workspace` -> PASS (exit 0)
