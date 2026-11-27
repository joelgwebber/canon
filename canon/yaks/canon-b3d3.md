---
id: canon-b3d3
title: 'TUI pass 3, step 3: the pane follows the cursor'
status: shaving
type: task
priority: 2
created: '2026-11-27T00:25:44Z'
updated: '2026-11-27T02:27:22Z'
parent: canon-3c5e
depends_on:
- canon-44f1
tags:
- tui
---

[The third pass](/doks/design/tui-places.md), The pane follows the cursor and Every kind has a detail. A detail per kind (track, album, artist, playlist), fetched after a short settle and cached; Enter goes in, p plays; Now playing is the playing entry's detail. Gathers canon-cb6a.

#### 2026-11-27T02:10:50Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-27T02:27:22Z

*herd-player*

Plan (coordinator, doing it in this thread):

- `App::follow_cursor`, run after every key, click and server message: the subject the cursor wants (Library/Sources: the selected item; a track is `Source::Track`; a Search entry is the last search of that service; Now: the queue entry's track, none on the playing entry, so Now playing). Fetched after `FOLLOW_DELAY` (150 ms), fired from `tick`; `Enter` fires it at once.
- Pages the pane lets go of go into a small cache (`follow_cache`, 24), taken back out on return: moving back is instant. Edits that reload playlists drop cached playlist pages.
- `Enter` in the list gives the pane the keys (no jump). In the pane, `Enter` re-subjects it (a jump, as before). `Esc` steps back out.
- `p` plays from here (track: its section from it; album/playlist: all of it; queue: that entry). `P`, `a`, `A` unchanged.
- Track detail: `ClientMessage::Track` (step 1): art, title, credits, duration, services with links; then Albums and Artists as rows.
- `.` jumps to Now, cursor on the playing entry. Queue `Esc` does the same.
- Headless settle waits out a due follow, so frames show the settled pane.
