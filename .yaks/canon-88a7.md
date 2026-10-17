---
id: canon-88a7
title: The great decoupling
status: shaving
type: idea
priority: 1
created: '2026-10-15T03:48:30Z'
updated: '2026-10-17T17:58:13Z'
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
