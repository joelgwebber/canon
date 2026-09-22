---
id: canon-c4c3
title: Decode/demux abstraction (symphonia fMP4 spike vs ffmpeg)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T03:33:31Z'
parent: canon-b192
labels:
- audio
- rust
verify: cargo test -p canon-audio
---

SPIKE FIRST, don't bet blind. Symphonia lists ISO/MP4 demux 'Great' and FLAC 'Excellent', but its README is silent on FRAGMENTED MP4 (moof/mdat) — which is exactly what Tidal ships. Verify symphonia can demux a real Tidal init+media fMP4 stream and decode FLAC/AAC from it. If it can't, fall back to an ffmpeg binding (ffmpeg-next), still statically linkable. Keep decode behind a trait so the backend is swappable. Resample with rubato (mixer-rate match in shared mode, passthrough when rates align). See the open question on this yak.

---
▸ 2026-09-22T02:01:40Z [claude]
Decode stack: pure-Rust symphonia (keeps the self-contained binary, but FRAGMENTED-MP4 support for Tidal's fMP4 is unverified and is the single riskiest assumption in the plan) vs an ffmpeg binding (proven demux, still static-linkable, but a heavy C dep). Preference before I run the fMP4 spike, or should the spike result decide it?

---
▸ 2026-09-22T02:34:44Z [Joel Webber]
Let's stick with Symphonia. If the fragmented mp4 codec is broken or missing, we can always go on a side quest to add it.

---
▸ 2026-09-22T03:33:30Z [Joel Webber]
verify: `cargo test -p canon-audio` -> PASS (exit 0)

---
▸ 2026-09-22T03:33:30Z [claude]
RESOLVED via spike -- risk RETIRED, no ffmpeg side-quest needed. Symphonia 0.6.1 (isomp4+flac+aac, default-features off) demuxes+decodes BOTH FLAC-in-fragmented-MP4 and AAC-in-fragmented-MP4 identically to a non-fragmented control (isomp4 gained first-class moof/mvex/trun/sidx fragmented support after 0.5, which is why the assumption was stale). fFLAC decoded 88200 frames = exactly 2.0s x 44100; fAAC matched the plain control's 90112 frames (rules out first-fragment-only demux). Built the decode seam (canon-audio/src/decode.rs): decode() over any Read+Seek+Send+Sync via a SeekableInput MediaSource adapter -> probe -> full packet/decode loop -> DecodeSummary(codec/rate/channels/bit_depth/frames/packets/errors). Reconciled the one cross-crate finding: canon_core::MediaInput now requires + Sync (Symphonia's MediaSource bound), so the production Box<dyn MediaInput> feeds decode() directly and the spike's temporary local trait is gone. Evidence: cargo test -p canon-audio (4 lib + 4 fMP4 integration tests over hermetic ~25KB assets) pass; clippy + fmt clean.
