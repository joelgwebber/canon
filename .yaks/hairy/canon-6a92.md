---
id: canon-6a92
title: Local files with the index at the end (non-streamable MP4) can't be opened
type: bug
priority: 2
created: '2026-09-23T20:41:21Z'
updated: '2026-09-26T21:22:47Z'
parent: canon-5cb2
labels:
- audio
- library
---

Found in the canon-7f16 cleanup (2026-09-23): `canon play-file crates/canon-audio/tests/assets/aac_plain.m4a` fails with "probe: unsupported feature: isomp4: missing moov atom" ("mp4 is not streamable"). The engine wraps every input in ForwardSource (is_seekable = false, byte_len None), which is right for Tidal segment streams, but an ordinary .m4a with its moov atom at the end needs a seek to read it. A local-file source must hand the engine a seekable input (SeekableInput from decode.rs already does this for the fMP4 check). Let MediaInput declare whether it is seekable, and only force-forward the streaming ones. Needed before the local file index (canon-5cb2) can play a real library.

---
▸ 2026-09-26T21:22:47Z [Joel Webber]
canon-5305 (2026-09-26) added the mechanism: a source can set ResolvedStream::seek_to and the engine then presents its input as seekable to Symphonia (length measured), which is exactly what a moov-at-end file needs. A local-file source should always open files as seekable (seek_to Some(start), including ZERO once the engine treats Some(ZERO) as seekable-from-start, since Decode::open currently filters a zero seek out).
