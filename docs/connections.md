# Connections and capabilities (design investigation, yak canon-699d)

Status: **proposal**. Nothing here is built. Research as of 2026-09-24; sources are listed at
the end, and several of the facts below changed within the last year, so expect them to change
again.

## 1. The problem

Services grant different things depending on *how* you log in:

- **Tidal.** A device-code login (the TV client) can browse, but our live test on 2026-09-22 got
  `401 subStatus 4005` on every playback request. The PKCE login (Android client, with the redirect
  URL pasted back) streams lossless and hi-res.
- **Spotify.** The Web API gives library and metadata access. Audio exists only through librespot,
  a reverse-engineered client that needs Premium.

canon doesn't model this, so choosing the wrong login breaks playback without saying why:

1. There is **one `ServiceSession` and one token file per service** (`tidal.json`), with an
   `is_pkce` flag inside it.
2. The API's `login_begin` / `login_poll` run the **device-code** flow, and PKCE exists only as
   the `canon login tidal --pkce` CLI. So a login done from a UI **overwrites the streaming token
   with one that can't stream**. Playback then fails with a raw `4005` from inside the controller.
3. `Sources` is keyed by `Service`: a single `TidalSource` is both "the Tidal catalog" and
   "the Tidal audio".
4. Six API ops default to `Service::Tidal` when no service is given.

Two requirements go beyond fixing this:
- You want **Tidal for audio** (you pay for it, and it pays artists better per stream).
- You want **Spotify for library, metadata, public playlists and recommendations**, and for the
  family members who use Spotify.

## 2. What each way in actually grants

| Service · login method | Catalog / metadata (ISRC) | User library read | Library write | Recommendations | Full audio | Constraints |
|---|---|---|---|---|---|---|
| Tidal v1 · PKCE (Android client) | yes | yes | yes | radio, mixes | **lossless / hi-res** | Unofficial, and Tidal keeps moving the gate (UA, client version, TLS fingerprint). Recent reports of PKCE handshake failures. |
| Tidal v1 · device code (TV client) | yes | yes | yes | yes | **none for us** (4005); others report AAC 320 only in 2026 | The quality ceiling is fixed per token, at login. |
| Tidal official v2 · auth code + PKCE (own app) | yes (incl. `filter[isrc]`, `filter[barcodeId]`) | yes | yes | daily, discovery and new-release mixes | **30 s previews** | Sanctioned. Full playback needs app review that Tidal hasn't granted anyone. |
| Tidal official v2 · client credentials | yes | — | — | — | — | No user involved: catalog only. |
| Spotify Web API · own dev-mode app | by id, ISRC/UPC (restored Mar 2026); search capped at 10 | saved tracks, *own and collaborative* playlists | yes | **no** (removed Nov 2024) | — | Owner needs Premium. Max **5 users**. Tokens die 6 months after consent. **Other users' and Spotify-owned playlists: metadata only, no tracks.** |
| Spotify Web API · a pre-2024 extended-quota client id | full read, incl. public playlists | yes | yes | legacy endpoints | — | Means borrowing another project's client id (ncspot does). Grey. |
| Spotify · librespot (OAuth, Spotify's own "keymaster" client id) | the token reaches the Web API but is heavily rate-limited (shared id) | — | — | — | **Ogg Vorbis, Premium only** | Unofficial. Audio-key refusals reported since Nov 2025. |
| Spotify Connect receiver (librespot zeroconf) | — | — | — | — | Audio for whoever is casting to us | **Inbound**: family members' Spotify apps would see canon as a speaker. Premium per account. Session-scoped. |

What this means for the design:

- **Capabilities attach to a login, not to a service.** The same service grants different things
  depending on the client id and the account's tier.
- **Capabilities must be verified, not just declared.** Declared (what the method should grant) is
  known before login. Verified is what a probe or real use has shown. Tidal's ceiling has moved
  three times in a year, and a token from last month may not still do what its method says.
- **Spotify covers less than hoped.** A personal dev-mode app gets your saved tracks and your own
  playlists (enough for your family to import theirs). It does **not** get public or editorial
  playlist contents, or recommendations. Getting those means one of:
  - borrowing an extended-quota client id;
  - scraping the web player's anonymous endpoints (tideway did that for play counts);
  - accepting the gap.
  That's a policy call for you (§7).

## 3. The model

### Capability

What canon can do through a connection:

```rust
enum Capability {
    Catalog,                          // search, album/artist listings, lookup by ISRC/UPC
    LibraryRead,                      // favorites, the user's playlists
    LibraryWrite,                     // export: save, write playlists
    Recommendations,                  // radio, similar, mixes
    Stream { max: Quality },          // full-length audio, up to a quality
    Preview,                          // 30 s clips (official Tidal v2); never auto-played
    Receiver,                         // others can play *to* canon (Spotify Connect)
}
```

### Connection

One authenticated way into one service for one account. **Several per service are normal**: your
Tidal PKCE login, your Spotify Web API login, a family member's Spotify login.

```rust
struct Connection {
    id: ConnectionId,                 // stable, canon-minted; names the credential file
    service: Service,
    method: MethodId,                 // "tidal.pkce", "tidal.device", "spotify.web", "spotify.librespot"
    account: Account,                 // who; `Account` already exists in canon-core
    declared: Capabilities,           // what the method should grant
    verified: Capabilities,           // what probes / real use have shown
    health: Health,                   // Ok | NeedsLogin | Degraded { why } | Failing { why }
}
```

### Connector

Replaces `ServiceSession`. There is one per service, and it describes its login methods as data,
so a client can present the choice and show what each method gets you before anyone logs in:

```rust
trait Connector {
    fn service(&self) -> Service;
    fn methods(&self) -> Vec<Method>;                  // id, label, flow kind, declared caps, caveats
    async fn begin(&self, method: &MethodId) -> Result<LoginFlow>;
    async fn complete(&self, flow: FlowId, input: FlowInput) -> Result<Connection>;
    async fn restore(&self, stored: StoredConnection) -> Result<ConnectionHandle>;
    async fn probe(&self, conn: &ConnectionHandle) -> Result<Capabilities>;
}

enum LoginFlow {
    DeviceCode { code: DeviceCode },                   // show a code, poll
    Browser { url: String, paste_back: bool },         // open a URL; PKCE pastes the redirect back
}
```

A `ConnectionHandle` hands out the traits canon already has, **gated by what was verified**:
- `catalog()` returns `Option<Arc<dyn Catalog>>`;
- `source()` returns `Option<Arc<dyn Source>>`;
- later, `library_writer()` and `receiver()`.

The existing `Source` / `Catalog` seams stay. What changes is who owns them and when they're
available.

### Routing

`Sources` becomes a view over connections, answering questions **by capability**:

- **"Which connection streams bindings on service S, best quality first?"** The controller uses
  this. A track's bindings are filtered to services with a streaming connection before any is
  tried.
- **"Which connection can browse S?"** Search, albums, artists and mixes use this. The Tidal
  default in six API ops becomes "the connection with `Catalog`", with an explicit `connection`
  parameter to override.
- A setting holds a **preference order for streaming** (Tidal first, say). Local files always win,
  as now.

When nothing qualifies, the answer is a typed error, not a service's raw status:

```rust
Error::NotEntitled { service, capability, hint }
// e.g. "Tidal is connected for library only. Reconnect Tidal with the streaming login to play."
```

### Probing and degrading

- **At login and on restore:** probe the declared capabilities cheaply. For Tidal, resolve
  `playbackinfo` for a known track at each quality to find the real ceiling.
- **At use:** a 4005 or 403 on playback turns into a downgrade of `verified` plus `health:
  Degraded`, rather than a one-off failure. The snapshot/API surfaces it, and routing falls through
  to the next connection or binding.

## 4. The payoff: playing what a service can't stream

Spotify-for-library plus Tidal-for-audio only works if canon can get from a Spotify binding to a
Tidal one. That is the library's job, and this design depends on it (yak canon-880e):

- Importing a Spotify playlist creates entities with **Spotify bindings and ISRCs**.
- Routing finds no streaming connection for Spotify, so the library **matches the entity to a
  streaming service by ISRC**, binds it (provenance `isrc`), and plays that.
  - The official Tidal v2 API has `filter[isrc]`.
  - Whether the v1 API has an equivalent needs checking; falling back to a search plus an ISRC
    check works either way.
- "Playable" becomes a derived property the API can show before you press play: *playable via
  Tidal*, *preview only*, or *no streaming match*.

This is also why bindings already carry provenance and confidence, and why ISRC was chosen as the
join key.

## 5. Accounts and the household

Several connections per service raise a question the code has avoided so far: **whose library is
it?** Today there is one library, one saved set, one queue.

Two options:
- **(a) One household library.** Everyone's imports land in it, with provenance recording which
  connection brought them in. Simple; fine if the family shares taste.
- **(b) Profiles.** Saved sets and playlists are per person; entities, bindings and matches are
  shared. Entity identity is already account-independent, so this stays cheap as long as "saved"
  and "playlists" gain an owner early.

## 6. Spotify Connect: a different kind of connection

A Connect receiver isn't a way for canon to reach Spotify. It's a way for **people's Spotify apps
to reach canon**:
- librespot in zeroconf mode shows up as a speaker to every Premium account on the LAN;
- it receives transport commands;
- it hands decoded PCM to canon, which could play it through its own outputs (Tunes, gapless flow).

For a Spotify household that is probably worth more than library import. It fits as a connection
with the `Receiver` capability, whose *queue is Spotify's*, not canon's. The player would need a
mode where an external controller owns the queue, and that is its own design.

## 7. Open questions for you

1. **Public and editorial Spotify playlists, and Spotify recommendations.** A personal dev-mode app
   can't read them. Accept the gap, borrow an extended-quota client id, or scrape the anonymous web
   endpoints? (For recommendations, Tidal's radio and mixes already work.)
2. **Household:** one library (5a) or profiles (5b)?
3. **Spotify for the family:** is the goal importing their libraries, or letting them cast to canon
   (Connect receiver, §6)? The two are different projects.
4. **Official Tidal v2:** use it for catalog, library and recommendations and keep v1 only for
   audio? It is sanctioned and more stable, but a second Tidal client id to register and a second
   JSON shape to model. Or keep v1 for everything until it breaks?

## 8. Proposed path (each step its own yak, nothing started)

1. **Connections in core, Tidal only.** `Capability`, `Connection`, `Connector` and method
   descriptors. Per-connection credential files, with the existing `tidal.json` migrated as
   `tidal.pkce` or `tidal.device` according to its `is_pkce`.
   - API ops `services` (methods and what they grant), `connect` / `connect_complete`,
     `connections`, `disconnect`. `login_*` retired.
   - Routing by capability; `NotEntitled` errors.
   - **Fixes the overwrite bug on its own.**
2. **Probing and degrading**, starting with Tidal's quality ceiling. Surface connection health in a
   (non-snapshot) `connections` reply.
3. **ISRC matching to a streaming connection** (canon-880e). This is what lets a non-streaming
   binding play.
4. **Spotify Web API connection** (`spotify.web`, own dev-mode app): catalog by id, saved tracks,
   own playlists, and import (reusing canon-4fb2's shape).
5. **Spotify audio via librespot** (`spotify.librespot`), if wanted: a `Source` behind a
   `Stream` capability.
6. **Spotify Connect receiver**, if wanted: its own design (external queue owner).

## Sources

- TIDAL API spec (v1.10.135): https://tidal-music.github.io/tidal-api-reference/tidal-api-oas.json
- TIDAL SDK auth: https://github.com/tidal-music/tidal-sdk/blob/main/Auth.md
- TIDAL ISRC lookup: https://github.com/orgs/tidal-music/discussions/26
- TIDAL full-playback review requests: https://github.com/orgs/tidal-music/discussions/214, https://github.com/orgs/tidal-music/discussions/179
- Token entitlements fixed at issue: https://dnim.dev/blog/tidal-24bit-pkce
- Device-flow quality downgrades: https://github.com/EbbLabs/python-tidal/issues/404, https://github.com/EbbLabs/mopidy-tidal/issues/247
- Spotify Nov 2024: https://developer.spotify.com/blog/2024-11-27-changes-to-the-web-api
- Spotify extended quota criteria (2025): https://developer.spotify.com/blog/2025-04-15-updating-the-criteria-for-web-api-extended-access
- Spotify Feb 2026 dev mode: https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide
- Spotify Mar 2026 (external_ids restored): https://developer.spotify.com/documentation/web-api/references/changes/march-2026
- Spotify quota (Jul 2026): https://developer.spotify.com/blog/2026-07-23-web-api-quota-updates
- librespot: https://github.com/librespot-org/librespot, options wiki, CHANGELOG; audio-key refusals #1649 / PR #1763
- Keymaster Web API rate limits: https://github.com/devgianlu/go-librespot/issues/282
- spotify_player's two-client pattern: https://github.com/aome510/spotify-player
