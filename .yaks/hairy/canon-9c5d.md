---
id: canon-9c5d
title: 'Playlist management: TUI parity for rename, delete, true duplicate, and live refresh'
type: task
priority: 1
created: '2026-10-01T13:07:37Z'
updated: '2026-10-01T13:07:37Z'
parent: canon-65f7
labels:
- tui
- library
---

Found while reviewing canon-5fe5: the TUI's playlist actions (canon-e55d) are missing several pieces the daemon API already supports, and one UI mechanism needed by more than one of them. None of this needs a canon-core/canon-library/canon-api change -- playlist_delete and playlist_rename already exist as ops; this is TUI wiring plus one small generalization (a reusable text prompt) that rename and a fixed 'copy' both need.

Splitting into: (1) a reusable text-prompt widget (today's self.input is hardwired to search), (2) rename a local playlist, (3) delete a local playlist with confirmation (canon-5fe5), (4) copying a local playlist should ask for a new name instead of silently colliding on the same one, (5) the current page refreshes after any action that changes what it's showing (copy/merge/create already silently go stale; rename/delete will too once built). (1) blocks (2) and (4). Creating a brand-new empty playlist from the TUI (today only canon control's 'pl new' can) is a related gap, not asked for -- left as an optional, low-priority sibling.
