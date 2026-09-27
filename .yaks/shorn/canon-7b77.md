---
id: canon-7b77
title: 'TUI phase 2: browse and queue from the library, playlists, search, albums and artists'
type: task
priority: 3
created: '2026-09-27T17:42:31Z'
updated: '2026-09-27T18:51:26Z'
parent: canon-3db9
labels:
- tui
verify: cargo test -p canon-tui
---

Tabs for the saved library (tracks/albums/artists), playlists, and search; Enter opens albums/artists/playlists; play now / next / append; save and unsave.

![tui-browse-session](artifacts/canon-7b77/tui-browse-session.txt)

![tui-album-session](artifacts/canon-7b77/tui-album-session.txt)

---
▸ 2026-09-27T18:51:22Z [Joel Webber]
Live (test daemon :7399, vol 0, real library, 2026-09-27; sessions attached): Library tab loaded 200/290 tracks, ] switched to Albums (229), Enter opened The Dark Side of the Moon (10 tracks); a => 'added "Speak to Me" to the queue' (queue 14→15, at the end), A => '"Speak to Me" plays next' (queue 16, inserted after the playing entry); Playlists (19) → Prog; Enter on its third track played the playlist from there (queue=2/14, 'Cold Wind to Valhalla'); / opeth → Tracks/Albums/Artists sections; * then * on 'Ghost of Perdition' => 'saved…' then 'removed … from the library' (net unchanged); an artist page (Pain Of Salvation) showed Top tracks + Releases; h went back. Tab bar steps down to shorter labels when narrow. 16 canon-tui tests (6 new for browsing), insta frames reviewed.

---
▸ 2026-09-27T18:51:25Z [Joel Webber]
verify: `cargo test -p canon-tui` -> PASS (exit 0)
