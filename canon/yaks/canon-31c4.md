---
id: canon-31c4
title: 'TUI: [ and ] listing switches are history jumps'
status: shorn
type: task
priority: 2
created: '2026-11-21T18:17:57Z'
updated: '2026-11-21T18:19:04Z'
parent: canon-3c5e
tags:
- tui
---

Joel (on canon-1c6e's landing): consistent history across states. Switching Library's kind or the Playlists shelf with [ / ] is a jump, so o returns to the previous listing with its cursor.

#### 2026-11-21T18:18:04Z

*herd-player · kind: move · from: hairy · to: shaving · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: hairy -> shaving

#### 2026-11-21T18:19:03Z

*herd-player*

Evidence: new test a_listing_switch_is_a_jump_and_o_brings_the_listing_back (2j ] → hist=2/0, kind Album; o → kind Track, cursor back, hist=1/1; i → Album). walked() now sets library_kind instead of pressing ] so the other history tests keep their counts. canon-tui 58 passed; green bar: fmt ok, clippy only the 2 old firewall.rs warnings, workspace tests ok, build ok.

#### 2026-11-21T18:19:04Z

*herd-player · kind: move · from: shaving · to: shorn · thread: ksQQ8ThW0PX0SAyxtEBqIr2N1ZLOAHSsupIBxBCA0aLJyrhJPreUvVwLftHG*

moved: shaving -> shorn
