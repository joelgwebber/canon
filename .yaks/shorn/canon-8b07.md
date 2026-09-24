---
id: canon-8b07
title: 'Spotify Web API crate: OAuth PKCE and a Catalog (search, albums, artists, saved library, own playlists)'
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:40:42Z'
parent: canon-b3a5
labels:
- spotify
verify: cargo test -p canon-spotify
---

Step 4. New canon-spotify crate against the Web API for a personal dev-mode app (Feb 2026 rules: search max 10 per request, single-item GETs, own/collab playlist contents only, external_ids restored Mar 2026). Implements canon_core::Catalog (radio/similar/mixes -> Unsupported) incl. favorites (saved tracks/albums/artists) and playlists for import, ISRC and UPC carried. Auth: authorization code + PKCE with a loopback redirect. Live use needs Joel's developer app client id.

---
▸ 2026-09-24T21:40:18Z [Joel Webber]
Built crates/canon-spotify (new crate, workspace member; reqwest 0.13 rustls+http2+form added to workspace.dependencies). Public API: SpotifySession::restore(Arc<dyn SpotifyHttp>, TokenStore, client_id) / with_redirect_uri / login_url / complete_login(redirect URL or bare code) / is_authenticated / account() (GET /me -> canon_core::Account) / track(id) / tracks_by_isrc(isrc) (search q=isrc:), and impl canon_core::Catalog for SpotifySession. SpotifyHttp trait + ReqwestHttp; TokenStore/PersistedTokens (atomic JSON, stores client_id so tokens from another app are ignored; a login in flight parks in <name>.pending.json so the two halves can run in different processes). No ServiceSession impl (Connector design replaces it).

---
▸ 2026-09-24T21:40:22Z [Joel Webber]
Decisions. Redirect URI: http://127.0.0.1:8898/spotify/callback (DEFAULT_REDIRECT_URI; Spotify refuses localhost, requires the loopback literal, http allowed for loopback). Nothing listens there; the user pastes the failed-page URL back (state is checked, error=access_denied surfaces). Scopes: the requested set PLUS user-follow-read, which GET /me/following needs for followed artists in favorites. Refresh: single-flight under the tokio Mutex, 60s skew, rotated refresh token captured else kept, invalid_grant clears the session (6-month expiry / revocation). 401 -> one forced refresh + retry. 429 -> wait Retry-After (default 1s) up to 3 times, any wait > 60s fails at once as Transient. 403 -> Unsupported (dev-mode forbids), 404 -> NotFound, 5xx -> Transient. next links are followed only if they start with https://api.spotify.com/v1 (the bearer never leaves the API host).

---
▸ 2026-09-24T21:40:27Z [Joel Webber]
Catalog. search: type=track,album,artist, pages 10 per request by offset up to min(limit,50), stops early when every kind returns a short page. album: GET /albums/{id} (embedded first tracks page + next links), then GET /tracks/{id} per track for ISRC when the album has <= 40 tracks (simplified album tracks lack external_ids and batch /tracks?ids= is gone; ISRC is what matches a Spotify binding onto a streaming service, so ~N GETs for a normal album is worth it; box sets skip it and can use track(id) later). artist: GET /artists/{id} + /artists/{id}/albums include_groups=album then single, 10/page (the new max), 50 each, sorted newest first; top_tracks empty. favorites: /me/tracks + /me/albums (added_at -> added_ms, ISRC/UPC carried), /me/following?type=artist cursor-paged via next (no added_at exists -> None). playlists: /me/playlists filtered to owner.id == /me id or collaborative, items from /playlists/{id}/items?additional_types=track (reads item and the deprecated track alias; skips episodes, local files, nulls); a 403 on one playlist skips it rather than failing the import. radio/similar_artists/mixes/mix -> Unsupported("... not available to Spotify development-mode apps"). Artwork = largest image by area. The Catalog trait on this branch has no tracks_by_isrc, so it is an inherent method.

---
▸ 2026-09-24T21:40:32Z [Joel Webber]
Cited (fetched 2026-09-24): developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide (search limit max 10 default 5; batch GETs, top-tracks, browse, /users/{id}/playlists removed; /playlists/{id}/tracks -> /items and items.items.track -> item; playlist contents only for owned/collaborative; /me loses country/product/email); references/changes/march-2026 (external_ids restored on track and album); tutorials/code-pkce-flow + tutorials/refreshing-tokens (authorize/token params, refresh may omit refresh_token -> keep old); concepts/redirect_uri (no localhost, 127.0.0.1 over http); reference pages get-playlists-items (limit 50, 403 when not owner/collaborator), get-users-saved-tracks (limit 50, added_at + full track), get-followed (cursor paging, user-follow-read, no added_at), get-a-list-of-current-users-playlists (items replaces tracks summary), get-an-artists-albums (limit max 10), get-an-albums-tracks (limit 50, simplified tracks lack external_ids), search (isrc:/upc: filters, limit 0-10, offset <= 1000).

---
▸ 2026-09-24T21:40:40Z [Joel Webber]
Evidence: UNIT TESTS ONLY. No live call was possible: the user has not registered a Spotify app yet. cargo test -p canon-spotify -> test result: ok. 34 passed; 0 failed (PKCE RFC 7636 appendix B vector, authorize URL, redirect/code parsing incl. state mismatch and access_denied, code exchange persisted across two processes, refresh keeps/rotates refresh token, single-flight refresh of 4 concurrent calls = 1 POST, invalid_grant signs out, 401 retry, foreign client id ignored, 429 Retry-After waits 14s virtual then succeeds, 3 retries then Transient, 3600s Retry-After fails at once, off-host next refused, fixtures for track/album/search paging/artist albums/favorites/playlists/isrc search/Unsupported). Green bar: fmt --check clean, clippy --workspace --all-targets zero warnings, test --workspace all ok, build --workspace ok. For live use the user must provide: a Spotify developer app (owner on Premium) with its client id, redirect URI http://127.0.0.1:8898/spotify/callback registered exactly, and each user (max 5) on the app allowlist. Field shapes are from the reference, not a live reply: the first live session should dump /me, /me/tracks, /me/playlists and one /playlists/{id}/items before trusting import (canon-6272).

---
▸ 2026-09-24T21:40:42Z [Joel Webber]
verify: `cargo test -p canon-spotify` -> PASS (exit 0)
