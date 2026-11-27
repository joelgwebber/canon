---
type: topic
title: 'The canon TUI, third pass: three places and a pane that follows'
created: '2026-11-21T22:00:00Z'
tags:
- tui
- design
- api
description: 'Now, Library and Sources replace the tabs; the pane shows the row under the cursor; every kind gets a local detail; one protocol behind TUI, CLI and MCP. Read before changing the TUI''s structure or adding a protocol op.'
---

# The canon TUI, third pass: three places and a pane that follows

Status: agreed with Joel, 2026-11-21. Tracked by [canon-3c5e](/canon/yaks/canon-3c5e.md). Follows
[the second pass](/doks/design/tui-player.md), whose six phases shipped (theme, command table,
list and pane, history, art, keys and mouse); its keys, history, art and theme stand. This dok
changes what the views are and what the pane follows, and it gathers the ideas in
[canon-603e](/canon/yaks/canon-603e.md), [canon-abaa](/canon/yaks/canon-abaa.md),
[canon-2b45](/canon/yaks/canon-2b45.md), [canon-2b75](/canon/yaks/canon-2b75.md),
[canon-cb6a](/canon/yaks/canon-cb6a.md) and [canon-5590](/canon/yaks/canon-5590.md) into one
design.

## Why

The second pass's views sort things by where they come from: a Playlists tab mixing canon's own
playlists with each service's, a Search tab that searches a service, `[` `]` shelves inside both.
That makes the user think about plumbing, and it stacks tabs inside tabs whose current choice is
hard to see. And the pane kept whatever was last opened, so a playlist opened in Playlists stayed
on show in the queue, with no way back to it and `Enter` doing two jobs (play, or open).

## Three places

The top-level distinction is what you are doing, not where the data lives:

| Place | Rows | `/` |
|---|---|---|
| **Now** | the queue | find in the queue |
| **Library** | what is yours: tracks, albums, artists, **playlists** | filter (on the server: the library is paged) |
| **Sources** | what comes from outside: each service's search, playlists and mixes; search everywhere; import and export | find in the list |

`1` `2` `3` and `Tab` move between them; Settings stays on `,`, outside the cycle. Library's kind
is a visible row of choices with the current one highlighted (`tracks · albums · artists ·
playlists`), switched with `[` `]`; it is the only second level left.

Sources' list is one list grouped by service, not a second row of tabs:

```text
 Tidal      Search
            Playlists
            Mixes
 Spotify    Search
            Playlists
 Everywhere Search
 Import / Export
```

The pane shows the row under the cursor, as everywhere else: a service's playlists, its mixes, a
search prompt with its last results. The list-and-pane model does the nesting.

## The pane follows the cursor

The pane shows the row under the list's cursor, as yaks' detail pane does: a track's detail, an
album, an artist, a playlist, a Sources entry. Moving the cursor changes the pane; it is never a
history step.

- **`Enter` goes in:** the keys move to the pane. Inside it, the pane's own cursor drives nothing;
  `Enter` on a row there makes that row the pane's subject, and that **is** a history step, so `o`
  undoes it. `Esc` gives the keys back to the list (and the pane follows the cursor again).
- **`p` plays from here** (a track: the list from it; an album or playlist: all of it). `P` plays
  now, `a` `A` enqueue at the end and next, as before. Expect to tune.
- **Now playing** is what the pane shows in Now when the cursor is on the playing entry; `.` puts it
  there from anywhere. One rule, no exception. A persistent couple of lines showing what is
  actually playing, with play/pause and position, can come back later as a global affordance; the
  transport strip is that today.
- The cost is a request per cursor move: fetch after a short settle (~150 ms) and cache per
  session; prefetch only if latency shows.
- **Narrow** (under 100 columns): no room for both, so the pane appears only after `Enter`, as now.

## Every kind has a detail, and it is local

Track, album, artist and playlist each get a canonical detail in the pane:

- **Track:** title, artists and album as links, duration, the services it is on, art.
- **Album:** art, credit, year, tracklist.
- **Artist:** top tracks, albums, similar artists where the service has them.
- **Playlist:** its tracks, and whose it is.

**Links are followed** (by key or click) through history. A detail is always the *local* one:
following a link to something not stored yet fetches it from a service and stores it, the way
search results already are (each ingested on arrival, a matching ISRC landing on the same track).
Stored is not saved: the library is what you saved, and `*` stays the way in.

Each detail lists the services that have it, each with its link (built by the daemon, never the
client) to copy.

## One protocol behind every interface

Each thing the TUI does is a protocol op; `canon control` and, later, MCP
([canon-c67f](/canon/yaks/canon-c67f.md)) call the same ops. No interface is more expressive than
another.

- **Replies that stream:** a reply can say more is coming (`more: true`), and a request's last
  reply says it is done. Search everywhere uses it first (one partial reply per service as each
  answers, then the merged final one); import progress and paged loads can follow. A client that
  wants only the end waits for the last reply.
- **Search everywhere is the server's:** `Search { service: None }` asks every connected service,
  so `control` and MCP get it too. Results merge by themselves, since a recording found on two
  services becomes one local track with two sources.
- **New ops and fields:** a track's detail; service links in views; Library's `query` used by the
  TUI.
- **A test that `control` reaches every op,** so the interfaces can't drift apart.
- **Undo** ([canon-1d6c](/canon/yaks/canon-1d6c.md)) as restoring a playlist's previous version,
  which canon already records: one key, one op.

## Decisions

Agreed with Joel on 2026-11-21:

1. **Three places: Now, Library, Sources.** Playlists are part of Library; services' things live in
   Sources.
2. **The pane follows the cursor,** everywhere, Now included; the cost (a request per move) is worth
   the clarity, papered over with a settle delay and a cache.
3. **`Enter` goes in, `p` plays.** Inside the pane, `Enter` re-subjects it and is a history step.
4. **A detail is always local;** following a link stores without saving.
5. **Sources is one grouped list,** not nested tabs.
6. **`/` in Library filters on the server;** elsewhere it finds in the loaded list.
7. **Search everywhere streams per service** through replies marked `more`, on the server.

## Steps

Each ships on its own, with headless frames (one rendered frame per change), `docs/tui.md` and
the protocol docs moving with it.

1. **Protocol:** streamed replies, a track detail op, service links in views, the `control`
   coverage test.
2. **Places:** Now / Library / Sources; playlists in Library; the visible kind row.
3. **The pane follows the cursor:** details for each kind, `p` to play, Now playing as the playing
   entry's detail.
4. **Links:** followed through history, stored on demand.
5. **Sources' contents:** the grouped list, search everywhere streamed, Library's filter.
6. **Undo** for playlists.
