---
id: canon-94cc
title: Source layer — Tidal integration
type: feature
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-23T04:32:53Z'
labels:
- tidal
verify: cargo test -p canon-tidal
---

Turn a track id into playable bytes: auth, stream resolution, the segment reader, the realtime bus, and play reporting. The whole surface is reverse-engineered and unversioned, so build the diagnostic surface (realtime status, play-report log) in from day one. Model behind canon-core's Source trait so local files and (later) other services slot in.

---
▸ 2026-09-22T02:01:39Z [claude]
Ban/ToS risk: canon relies on the same reverse-engineered, unversioned Tidal surface tideway does (Pushkin realtime, event-batch play reporting, TLS-fingerprint impersonation, the two hardcoded client-ids). tideway accepts this as a personal-use tool. Do you accept the same posture for canon (personal use, degrade-honestly, diagnostic-first), and is any of it a non-starter you'd want gated behind an off-by-default flag?

---
▸ 2026-09-22T02:44:44Z [Joel Webber]
Yup. Unfortunate, but unless and until they have a "real" API, we're stuck with it.

---
▸ 2026-09-23T04:32:45Z [Joel Webber]
verify: `cargo test -p canon-tidal` -> PASS (exit 0)

---
▸ 2026-09-23T04:32:53Z [Joel Webber]
All five remaining children shorn: 8bab (auth + rotating refresh, single-flight), d389 (PKCE for hi-res entitlement), a880 (HTTP + TLS-fingerprint impersonation via wreq), 4c55 (MPD -> segment list + manifest cache), e99d (segment reader as MediaSource + expiry re-resolution). canon-333e was hoisted to top-level rather than shorn -- see its note; it is a presence feature that speaks Tidal, not part of sourcing.

The deliverable was 'Tidal as a data source: authenticate, resolve a stream, hand us bytes', and it is proven end to end rather than by unit tests alone: PKCE login, then hi-res FLAC streamed from Tidal, decoded, and played both to local output and to a KEF LS50 II over Chromecast, with in-track seek re-resolving the stream at a segment boundary.

What this buys architecturally is the thing the project is actually about: Tidal is now behind the Source trait, so it is one data source among future others rather than the shape of the daemon.
