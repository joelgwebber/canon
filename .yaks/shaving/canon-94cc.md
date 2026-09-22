---
id: canon-94cc
title: Source layer — Tidal integration
type: feature
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T21:32:35Z'
labels:
- tidal
---

Turn a track id into playable bytes: auth, stream resolution, the segment reader, the realtime bus, and play reporting. The whole surface is reverse-engineered and unversioned, so build the diagnostic surface (realtime status, play-report log) in from day one. Model behind canon-core's Source trait so local files and (later) other services slot in.

---
▸ 2026-09-22T02:01:39Z [claude]
Ban/ToS risk: canon relies on the same reverse-engineered, unversioned Tidal surface tideway does (Pushkin realtime, event-batch play reporting, TLS-fingerprint impersonation, the two hardcoded client-ids). tideway accepts this as a personal-use tool. Do you accept the same posture for canon (personal use, degrade-honestly, diagnostic-first), and is any of it a non-starter you'd want gated behind an off-by-default flag?

---
▸ 2026-09-22T02:44:44Z [Joel Webber]
Yup. Unfortunate, but unless and until they have a "real" API, we're stuck with it.
