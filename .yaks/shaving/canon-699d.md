---
id: canon-699d
title: 'Design: connections with capabilities (library/metadata vs streaming access per service)'
type: task
priority: 2
created: '2026-09-24T19:12:48Z'
updated: '2026-09-24T19:19:35Z'
parent: canon-1175
labels:
- arch
- design
- spotify
- tidal
needs: human
---

Investigation, no implementation. Services split access by login method: Tidal device-code tokens browse but 4005 on playback while PKCE (Android client) tokens stream; Spotify's Web API gives library/metadata and librespot (Premium) gives audio. Today canon has one ServiceSession per Service, one token slot per service (a device-code login from the API overwrites the PKCE token and silently breaks playback), and the Sources registry keyed by Service. Formalize it: first-class connections (service + account + login method) that carry what they grant (catalog, user library, recommendations, streaming up to a quality, ...), so the core knows what each connected service can do, routes by capability, and says why instead of failing. Covers: Joel wanting Tidal for audio and Spotify for library/public playlists/recommendations, and family members on Spotify.

---
▸ 2026-09-24T19:19:35Z [Joel Webber]
Investigation written up in docs/connections.md. Findings: capabilities attach to a login method + account, not a service, and must be verified by probing (Tidal moved its quality gate three times in a year; entitlements are fixed per token). Spotify dev-mode apps (Feb 2026: owner needs Premium, max 5 users, 6-month tokens) get saved tracks and own playlists with ISRC, but NOT public/editorial playlist contents or recommendations. librespot = Premium audio; its keymaster token reaches the Web API but is rate-limited. Connect receiver = inbound, a different kind of connection. Proposal: Capability / Connection / Connector (methods as data, LoginFlow DeviceCode|Browser) / capability routing with Error::NotEntitled / probe+degrade; the cross-service payoff rests on ISRC matching to a streaming connection (canon-880e). Step 1 alone fixes the token-overwrite bug.

---
▸ 2026-09-24T19:19:35Z [Joel Webber]
Open questions (docs/connections.md section 7): (1) Spotify public/editorial playlists + recommendations are closed to personal dev-mode apps: accept the gap, borrow an extended-quota client id, or scrape anonymous endpoints? (2) Household: one shared library, or per-person profiles (saved + playlists owned)? (3) Family on Spotify: import their libraries, or let them cast to canon as a Spotify Connect receiver? (4) Use the official Tidal v2 API for catalog/library/recommendations and keep v1 only for audio, or stay on v1?
