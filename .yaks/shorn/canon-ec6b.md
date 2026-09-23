---
id: canon-ec6b
title: 'Tidal mixes: list My Mixes / Daily Discovery, show and play one'
type: task
priority: 2
created: '2026-09-23T22:11:50Z'
updated: '2026-09-23T22:15:02Z'
parent: canon-3842
labels:
- library
- tidal
- api
verify: cargo test -p canon-library a_mix_is_listed && cargo test -p canon-tidal the_mixes_page
---

The account's personal mixes are the richest recommendations Tidal offers. /v1/pages/my_collection_my_mixes lists them (MIX_LIST module; skip video mixes); /v1/mixes/<id>/items gives the tracks (same shape as playlist items). Catalog::mixes/mix; ItemRef gains {service, mix} so a mix queues like any item; ops mixes and mix; canon control mixes / mix #n / play #n. Mixes change daily, so they are not stored as playlists (a user can pl fromqueue one).

---
▸ 2026-09-23T22:14:59Z [Joel Webber]
Built: Catalog::mixes/mix + SourceMix; Tidal over /v1/pages/my_collection_my_mixes (MIX_LIST modules, video mixes skipped) and /v1/mixes/<id>/items; ItemRef::Mix {service, mix} expands to the mix tracks (ingested) and is refused where an entity is needed; Library::mixes/mix; ops mixes, mix; canon control mixes, mix #n, play #n. Also canon tidal-get <path> [k=v...], a diagnostic that prints any authenticated Tidal reply (how the mix endpoints were read before modelling).

---
▸ 2026-09-23T22:14:59Z [Joel Webber]
Live 2026-09-23: mixes -> My Daily Discovery + My Mix 1..8 (video mixes left out); mix #2 -> My Mix 1, 40 tracks (Maybeshewill, Meniscus, Jakob, Red Sparowes...); play #1 -> "playing 0:03/6:20 Angel — Massive Attack [1/10]".

---
▸ 2026-09-23T22:15:02Z [Joel Webber]
verify: `cargo test -p canon-library a_mix_is_listed && cargo test -p canon-tidal the_mixes_page` -> PASS (exit 0)
