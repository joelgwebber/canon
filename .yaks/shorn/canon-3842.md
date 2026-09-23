---
id: canon-3842
title: 'Recommendations: track and artist radio, similar artists, autoplay at the end of the queue'
type: task
priority: 2
created: '2026-09-23T21:57:32Z'
updated: '2026-09-23T22:02:44Z'
parent: canon-4185
labels:
- library
- api
- tidal
verify: cargo test -p canon-library radio && cargo test -p canon-core settings && cargo test -p canon-daemon autoplay
---

Catalog::radio and similar_artists exist (canon-b989). Surface them: radio {item} (a track or artist) -> tracks as library views, similar {item} -> artists; queue_add can take a radio. Autoplay: an opt-in setting that, when the queue runs out, extends it with radio seeded from the last track, so playback never just stops.

---
▸ 2026-09-23T22:02:38Z [Joel Webber]
Built: Library::radio (track or artist seed -> ingested track ids), similar (artists), track_views/track_refs; ops radio/similar (ReplyData::Tracks/Artists); Settings.queue.autoplay (off by default, omitted from the file when default); canon-daemon autoplay.rs: when the last entry is playing/loading with repeat off, radio seeded from it is appended (up to 25 not already queued), one attempt per (track, queue length). canon control: radio [item], similar <artist>, autoplay on|off.

---
▸ 2026-09-23T22:02:38Z [Joel Webber]
Live 2026-09-23: radio (current track Army of Me) -> 50 tracks (Portishead, Tricky, Hole, Björk...); similar 9706 -> Jethro Tull, Eric Clapton, Renaissance, ... ; autoplay on, play 55391796 (Eclipse only) -> serve.log "autoplay: adding 25 tracks" as it started, queue grew to 26; seek 1:58 -> after Eclipse "> 2. Subterranean Homesick Alien". autoplay off restored settings.json byte-for-byte (no queue key).

---
▸ 2026-09-23T22:02:44Z [Joel Webber]
verify: `cargo test -p canon-library radio && cargo test -p canon-core settings && cargo test -p canon-daemon autoplay` -> PASS (exit 0)
