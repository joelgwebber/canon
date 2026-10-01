---
id: canon-65f7
title: Import/export + track matching
type: task
priority: 3
created: '2026-09-22T02:01:39Z'
updated: '2026-10-01T01:21:14Z'
parent: canon-4185
labels:
- library
---

Import playlists (Spotify/Deezer/M3U) by normalising rows to {name, artists, duration, isrc} and resolving to canon entities via the F2 matcher — landing in a canon playlist, NOT a new Tidal playlist. Add the reverse tideway never had: EXPORT a canon playlist to a service, honouring each service's quirks. This is the 'own your library organization' payoff.

---
▸ 2026-10-01T01:21:14Z [Joel Webber]
Design settled 2026-09-30 after discussion with Joel. Replaces the old plan below (bulk import binding+overwriting upstream playlists locally).

Model: upstream playlists and mixes are exposed read-only behind the existing ItemRef facade (service item by kind, or mix) -- same shape as a local playlist, so a client can't tell a local source from a remote one. Two ops on any source: COPY (create_playlist from it) and MERGE (playlist_add into an existing local playlist, append-only, deduped by recording identity so re-merging an upstream playlist only pulls in what's new). EXPORT is create-new-only: make a new upstream playlist from a local one's tracks, stamped with a description noting when it was exported from canon. Re-export means nuke the old one by hand and export again -- canon never deletes upstream. No mirrors, no cache, no automatic upstream history; copy is how you keep something, and local history (once built) is how you undo a bad merge.

Two decisions: (1) canon never deletes anything upstream, at least at first -- export stamps a description instead. (2) bulk import (canon-4fb2) stops touching playlists; existing bound imported playlists become ordinary local playlists via a migration that drops the binding and otherwise changes nothing. Favorites import is unaffected except for its own known bug (re-saves locally-unsaved items).

Splitting into child yaks, in build order: unbind+stop-importing-playlists, history, track-sources (ItemRef expansion + Catalog::playlist(id) + list service playlists/mixes), merge, TUI browsing+copy/merge, export, search (playlists in SearchResults + open by URL), favorites re-import fix. Diff, in-playlist dedupe, and an export log (to support deleting canon's own upstream copies later) are explicitly deferred.
