---
id: canon-6272
title: Spotify as a connection (spotify.web) wired into the daemon
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:55:45Z'
parent: canon-b3a5
depends_on:
- canon-c739
- canon-8b07
labels:
- spotify
verify: cargo test -p canon-spotify connector && cargo test -p canon-core settings
---

Wire the Spotify crate in as a connector with method spotify.web (Catalog, LibraryRead, LibraryWrite; no Stream), so import and browsing work against Spotify and its tracks play via ISRC matching (step 3b).

---
▸ 2026-09-24T21:55:33Z [Joel Webber]
Built SpotifyConnector (crates/canon-spotify/src/connector.rs), method spotify.web: flow Browser (paste back the URL landed on; redirect http://127.0.0.1:8898/spotify/callback), grants catalog + library_read + library_write, no recommendations, no stream. Credentials <state_dir>/spotify.web.json (pending login in spotify.web.pending.json). source(Catalog) -> SpotifySource whose describe = SpotifySession::track and open = Error::NotEntitled{Spotify, Stream}; source(Stream) always None (so Sources::open on a Spotify-only track reports NotEntitled with the hint until ISRC matching binds it elsewhere). hint(Stream) says Spotify tracks play through a streaming service by ISRC match and that Spotify audio would need librespot (not built). Health via account() (GET /me) like Tidal. Added SpotifySession::forget and TokenStore::clear for disconnect.

---
▸ 2026-09-24T21:55:33Z [Joel Webber]
Client id: new settings key spotify.client_id (canon_core::SpotifySettings { client_id: Option<String> }, skipped when default, deny_unknown_fields, round-trip test). Decision: the connector holds Arc<dyn SettingsStore> and re-reads the id on every begin/complete/disconnect/connections (i.e. every `services` listing), restoring the session for the new id when it changed, so setting it needs NO restart. Sync grants/source/catalog use the session as last restored. Without an id, methods() still lists spotify.web with a "Not set up: ..." note, and begin fails with Error::Auth naming developer.spotify.com, Premium, the redirect URI and spotify.client_id. Daemon: run_serve loads settings before sources and registers Sources::new().with_connector(tidal).with_connector(spotify) (Tidal first = default browse service). canon login stays Tidal-only. canon control gains `spotify-app <client-id>` (sets the setting) and `import [service]`. import/search {service:"spotify"} route through Sources::catalog(Spotify) unchanged.

---
▸ 2026-09-24T21:55:33Z [Joel Webber]
Live check (test daemon, empty --state-dir target/live-spotify, --bind 127.0.0.1:7399; no Spotify app is registered, so no real login was possible). printf "sleep 2\nservices\nconnect spotify.web\nimport spotify\nspotify-app demo-client-id\nconnect spotify.web\nsettings\nquit\n" | canon control --connect 127.0.0.1:7399 --quiet ->
tidal
  tidal.pkce       Tidal (browser login)            not signed in
                   grants: browse, library, library changes, recommendations, streaming up to hi_res
  tidal.device     Tidal (code on another device)   not signed in
                   grants: browse, library, library changes, recommendations, no streaming
spotify
  spotify.web      Spotify (browser login)          not signed in
                   grants: browse, library, library changes, no streaming
error: auth: no Spotify client id: register an app at developer.spotify.com (its owner needs Premium), add http://127.0.0.1:8898/spotify/callback as its redirect URI, then set spotify.client_id in settings.json (or with set_settings) to the app's client id
error: spotify can't browse: no Spotify client id: register an app at ... (same hint)
spotify app demo-client-id: `connect spotify.web` to sign in
Open this URL in a browser and log in:
  https://accounts.spotify.com/authorize?response_type=code&client_id=demo-client-id&scope=user-library-read%20...&redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fspotify%2Fcallback&code_challenge_method=S256&code_challenge=...&state=...
{ "spotify": { "client_id": "demo-client-id" } }
State dir afterwards: library.sqlite, settings.json, spotify.web.pending.json. So: both services listed; the no-id error is actionable; import through the generic path reports NotEntitled with the hint; setting the id live (no restart) makes connect produce an authorize URL for it. NOT verified: a real Spotify login, /me, import against real data (needs the user's developer app; first live session should dump /me, /me/tracks, /me/playlists per canon-8b07).

---
▸ 2026-09-24T21:55:40Z [Joel Webber]
verify: `cargo test -p canon-spotify connector && cargo test -p canon-core settings` -> PASS (exit 0)

---
▸ 2026-09-24T21:55:45Z [Joel Webber]
Green bar: cargo fmt --all --check clean; cargo clippy --workspace --all-targets zero warnings; cargo test --workspace all ok (canon-spotify 41 passed incl. 7 connector tests; canon-core 68 passed); cargo build --workspace ok.
