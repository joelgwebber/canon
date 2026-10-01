---
id: canon-65f7
title: Import/export + track matching
type: task
priority: 3
created: '2026-09-22T02:01:39Z'
updated: '2026-10-01T13:11:38Z'
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

---
▸ 2026-10-01T02:09:56Z [Joel Webber]
BLOCKER found from the canon-4b3b lane, not caused by it: Joel's real library will not open, so `canon serve` on the shared state dir exits at startup.

  Error: "open the library at /Users/joel/Library/Application Support/canon/library.sqlite:
  library: table playlist_versions already exists ... at offset 18"

sqlite3 on that file: `PRAGMA user_version` is 8, yet playlist_versions, playlist_version_tracks
and playlist_version_tracks_track are all already there. Migration 9 is canon-120d's, so its DDL
has run against that database but the version counter says it has not, and every start now
re-runs it and fails. migrate() itself is sound (one transaction per migration, pragma bumped
inside it), so the likely cause is parallel-lane migration renumbering: canon-120d's migration
sat at a lower index in its own worktree, a build from there was run against the real library,
and landing canon-ae6e/canon-f917 ahead of it shifted it down one. Any database already migrated
by a lane build is in this state.

Two things for whoever owns this: (1) un-break the live database -- `PRAGMA user_version = 9` is
exactly what the migration would have left behind, every object of v9 being present, but it is
Joel's library so it is his call, not an agent's; (2) decide whether parallel lanes may add
migrations at all, or whether migrations must be made re-runnable (IF NOT EXISTS) / claimed by
index up front. A fifth bug class to add to the hardware list: a lane build run against the real
state directory can leave it unopenable by main.

Consequence for canon-4b3b: merge is covered by unit tests and a signed-out wire probe only; no
end-to-end merge against real Tidal data was possible.
