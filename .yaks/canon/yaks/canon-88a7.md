---
id: canon-88a7
title: The great decoupling
status: shaving
type: idea
priority: 1
created: '2026-10-15T03:48:30Z'
updated: '2026-10-18T15:38:15Z'
source: arch
---

The Big Idea: Break the canon server into two parts:
- One whose job it is to connect to upstream data sources, maintain the database, and serve the API used by players.
- A player, whose job it is to connect to the server, get playlists, tracks, etc, and play them on a local device, including outputs like network speakers.

This would imply dropping the TUI as its own thing, and merging it into the player (or having multiple player binaries that share most code outside their UIs).

This approach has several advantages:
- A shared database for track information, playlists, queues, etc.
- Players don't need their own credentials -- the server forwards tracks directly.
- It should be easier to create more players for different environments, because they only have to deal with fetching data and playing tracks. Eg, native mobile apps, a desktop UI, etc.

---
▸ 2026-10-17T17:58:13Z [Joel Webber]

moved: hairy -> shaving

---
▸ 2026-10-17T19:15:56Z [delta:The Great Decoupling]

Decision: design settled with Joel (thread "The great decoupling"). Children carry the work.

**Cut A, at the `Effect`/`EngineEvent` seam — not at the `Source` seam.**
- The server keeps the library, connectors/credentials, MusicBrainz, autoplay, and **one player actor (queue, generations, snapshot) per player**. arch.md rule 3 ("clients never hold playback logic; the queue lives on the server") stays true as written.
- A player is the executor: today's `PlaybackController`, `canon-audio`, and `canon-sink`. It owns its local output, discovery, and Cast/DLNA sessions; the server never touches a speaker. It reports its discovered outputs up; `SelectSink` stays a command to its actor and comes back down as an effect.
- Rejected: Cut B (player owns queue + actor, server only proxies audio). It reverses rule 3, makes every native player re-implement queue logic, and needs its own remote-control path.

**Roles on the wire.** *Executor* (receives effects, sends engine events) and *controller* (sends commands, renders snapshots). Player + UI in one process is the primary model and holds both roles; a headless player holds only executor; a remote UI only controller. Splitting them later is packaging the server never sees. Two players per user must be possible: a player has a stable id, and reconnecting with it resumes its own actor and queue.

**Audio to players:** the server serves FLAC at the source's native bit depth over plain HTTP. The 16-bit cap (`FLAC_BITS`) is a renderer rule and stays on the player. A lossy tier (AAC for iOS, Opus elsewhere) is a later per-player quality setting. Seeks are a fresh stream from the position, as renderers already do; byte ranges / HLS wait for a native iOS player.

**Auth from day one**, used on the LAN too, so internet exposure later is TLS in front rather than a second code path: long-lived player tokens (which double as the player id), short-lived signed tokens in audio URLs (native players want a bare URL). The player's own speaker-facing stream server stays unauthenticated on its LAN.

**Network loss:** players retry streams and the WS with backoff, then give up with the existing `EngineEvent::Failed`. The actor outlives the connection; a lost player shows as disconnected/paused, with no fail-back to local. Offline downloads: later, nothing here blocks them.

**Accepted costs / consequences**
- Speakers become per-player: closing a laptop that drives Kitchen stops Kitchen. The always-on box runs the server plus a headless player.
- The server needs a LAN bind, a firewall rule (the canon-6227 startup check moves with it), and auth. Cast/DLNA firewall/multicast issues become each player's environment problem.
- Local-output position stops flowing through the shared `Arc<FrameClock>` and arrives as reports: a third case for `position.rs`; needs hardware checking.
- Renderer path decodes and re-encodes FLAC twice. Cheap; accepted rather than special-casing passthrough. The codec survey is its own later yak.

---
▸ 2026-10-17T19:15:56Z [delta:The Great Decoupling]

Children, in rough order: canon-e329 (protocol crate) and canon-973d (actor per player) first; canon-806d (executor role) on both; canon-040e (FLAC endpoint), canon-5ce4 (position as reports) and canon-65d2 (auth) alongside; canon-1ad8 (the first player) on all of those; canon-7196 (headless player) after it. canon-3f01 (codec survey) is later and blocks nothing.

---
▸ 2026-10-18T15:38:15Z [delta:The Great Decoupling]

State of play, 2026-10-18 (all on main through 343044c; 19 commits past 321bd13):

Done and verified on hardware: protocol crate (e329), actor per player + persisted queues (973d), executor seam (806d: Tunes over Cast and DLNA, seek/pause/protocol switch, back to local), local position as reports (5ce4), server FLAC endpoint (040e: Tidal from 60s, ffprobe/flac ok), auth (65d2: paired tokens off loopback, signed audio URLs), `canon player` (1ad8: remote executor + TUI on one connection, server restart resumes; one-connection select deadlock found by Joel and fixed), player logs off the TUI (394c), codec survey (3f01).

Joel uses serve + player day to day; switching across devices works.

Open under this yak:
- canon-7196: `canon player --headless` exists; untested on the real always-on box / a box with no audio device.
- canon-934b: players never stream Tidal hi-res (controllers open at Lossless).
- SSDP fallback warning and a no-output serve mode: filed as children today.
Not verified anywhere: a player on a second machine; the librespot path through /audio (no librespot login on the server).
