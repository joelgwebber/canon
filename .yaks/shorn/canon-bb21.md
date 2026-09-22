---
id: canon-bb21
title: Interactive ws client (TUI/CLI) for driving + stress-testing
type: task
priority: 2
created: '2026-09-22T21:12:04Z'
updated: '2026-09-22T21:23:00Z'
parent: canon-1190
labels:
- api,tui
verify: cargo test --workspace
---

A simple interactive client over the ws control plane: connect, show the live snapshot (state/track/position/duration), and issue play/pause/next/prev/seek/enqueue/volume from keypresses or a REPL. The hands-on way to stress-test transport + queue + seek without hand-rolling Python each time.

---
▸ 2026-09-22T21:22:48Z [Joel Webber]
Shorn: canon control, an interactive keypress client over the ws plane. Enqueues track-id args (also the 1-9 palette), shows a live status line (state/track/pos/dur/vol), and maps space/n/p/[ ]/+ -/m/s/c/q to commands. Built + --help verified; raw-mode restored on exit via a Drop guard. Hands-on stress-test tool for the upcoming pass.

---
▸ 2026-09-22T21:23:00Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)
