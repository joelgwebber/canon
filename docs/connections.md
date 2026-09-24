# Connections and capabilities (design investigation, yak canon-699d)

Status: **agreed design** (2026-09-24), being built in the steps of §8. Research as of
2026-09-24; sources are listed at the end, and several of the facts below changed within the last
year, so expect them to change again.

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
  Decided: accept the gap (§7).

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

One authenticated way into one service. **One account per instance**: a canon instance holds at
most one connection per (service, method), so a connection is named by that pair (`tidal.pkce`,
`spotify.web`, `spotify.librespot`) and needs no minted id. Spotify still gets two connections
from one account, because its library and its audio are different ways in.

```rust
struct Connection {
    id: ConnectionId,                 // (service, method); names the credential file
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

## 5. Accounts

**Decided: one account per service per instance.** One library, one saved set, one queue. Entity
identity is account-independent, so per-person profiles stay possible later if "saved" and
"playlists" ever gain an owner.

## 6. Spotify Connect: two ways to use it

Connect speakers stream for themselves: the controlling app is only a remote, and the speaker
holds its own Spotify session and fetches the audio. That is why playback survives every phone app
closing, and why audio can't be redirected elsewhere; it goes to whichever device plays it. So
there are two ways to use Connect:

- **Control an existing Connect speaker** (Tunes almost certainly is one). With Premium, the Web
  API's player endpoints tell a Connect device what to play, and it streams from Spotify itself.
  - Entirely sanctioned.
  - The audio never passes through canon: no local output, no flow mode, no gapless mixing with
    Tidal tracks, and state has to be polled.
  - A third kind of Spotify connection (a remote for a speaker that plays Spotify itself). The same
    pattern would work for Tidal Connect.
  - Unverified: whether the February 2026 changes left the player endpoints open to dev-mode apps.
- **Be the Connect speaker** (librespot), described next.

### canon as a Connect receiver (later)

A Connect receiver isn't a way for canon to reach Spotify. It's a way for **people's Spotify apps
to reach canon**:
- librespot in zeroconf mode shows up as a speaker to every Premium account on the LAN;
- it receives transport commands;
- it hands decoded PCM to canon, which could play it through its own outputs (Tunes, gapless flow).

For a Spotify household that is probably worth more than library import. It fits as a connection
with the `Receiver` capability, whose *queue is Spotify's*, not canon's. The player would need a
mode where an external controller owns the queue, and that is its own design.

## 7. Decisions (2026-09-24)

1. **Spotify public/editorial playlists and recommendations:** accept the gap. A personal
   dev-mode app's own library and playlists are enough; Tidal covers recommendations.
2. **Accounts:** one per service per instance (§5).
3. **Family:** out of scope for now.
4. **Official Tidal v2:** a later evolution path, for when v1 breaks or matching wants its ISRC
   lookup. v1 stays for everything today.
5. **Spotify audio:** spike librespot as a `Source` (audio through canon), and test it carefully:
   audio-key refusals have been reported since Nov 2025. Fallback: control Connect speakers (§6).

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
   - (a) The mechanism: match an entity onto a service by ISRC and bind it. Independent of step 1.
   - (b) Wiring into playback: no streaming binding means match first. Needs step 1's routing.
4. **Spotify Web API connection** (`spotify.web`, own dev-mode app): catalog by id, saved tracks,
   own playlists, and import (reusing canon-4fb2's shape). The crate and its `Catalog` are
   independent of step 1; becoming a connection needs it. Live use needs a Spotify developer app,
   and the app owner on Premium.
5. **Spotify audio: a librespot spike** (`spotify.librespot`), a `Source` behind a `Stream`
   capability, with controlling Connect speakers (§6) as the fallback.
6. Later: official Tidal v2 for library access; canon as a Connect receiver.

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
