---
id: canon-3118
title: Service playlist views can be quite slow
type: bug
priority: 2
created: '2026-10-01T11:40:40Z'
updated: '2026-10-01T11:52:31Z'
labels:
- library
- playlist
verify: cargo test -p canon-spotify catalog::
---

Spotify's is particularly bad, taking ~30s at times to fetch remote playlists every time they're requested. At a minimum, we need some degree of local caching, without adding any more complexity than necessary (the fact that they're read-only should help).

Even better if we can find a way to isolate updates to changes, if the service APIs provide enough surface area to make that tractable.

---
▸ 2026-10-01T11:52:21Z [Joel Webber]
verify: `cargo test -p canon-spotify catalog::` -> PASS (exit 0)

---
▸ 2026-10-01T11:52:28Z [Joel Webber]
Fixed by not fetching tracks eagerly, not by caching. canon-spotify Catalog::playlists() now describes each playlist from the /me/playlists listing alone (name + track_count from the total Spotify already sends), with zero /playlists/{id}/items calls — the sole real caller, Library::service_playlists, only ever read name/track_count and discarded .tracks. Added a Catalog::playlist() override (mirroring canon-tidal) that fetches one playlist by id directly instead of falling through the trait default (which called playlists() and fetched every playlist in full just to pick one out). Net: browsing N playlists costs 1 paginated listing call instead of N+1; opening one costs 1-2 calls instead of listing-everything. A playlist marked collaborative but not actually shared can no longer be silently filtered out of the browse list (the listing alone cant tell); it now surfaces as an error only if opened, which is rare and more honest than hiding it. Full green bar passes: fmt --check, clippy --workspace --all-targets (0 warnings), test --workspace, build --workspace.
