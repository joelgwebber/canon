---
id: canon-581d
title: Create a new empty playlist from the TUI
type: task
priority: 3
created: '2026-10-01T13:08:30Z'
updated: '2026-10-01T13:08:30Z'
parent: canon-9c5d
depends_on:
- canon-4c6e
labels:
- tui
---

Not asked for -- a related gap noticed while scoping canon-9c5d. Today only canon control's 'pl new <name>' can make an empty playlist; the TUI's only playlist-creation path is copying a source into a new one (c). Once canon-4c6e's prompt exists, a key on the Playlists tab's own 'mine' listing (not on a row) could call playlist_create with an empty item list and the typed name. Low priority; do it only if it falls out cheaply alongside the others, or defer.
