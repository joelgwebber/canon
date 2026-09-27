---
id: canon-3db9
title: Canonical interactive TUI client
type: feature
priority: 3
created: '2026-09-27T17:16:32Z'
updated: '2026-09-27T19:44:50Z'
labels:
- tui
verify: cargo test -p canon-tui
---

We have the `canon control` CLI for driving the client with a simple stdin/out interface, which is perfect sense for agent-driven and scripted testing. Now I'd like to layer in a more interactive TUI client, using the same approaches we do in the yaks project.

Please start by diving into the details of the `yaks tui` implementation, and you'll find a rich set of ideas and primitives for doing interactive development of rich TUI apps, using the upstream `ratatui` crate, and our own `toque` crate for headless rendering, scripted interactions, and testing infrastructure. If you find it useful, we can also take a side-quest to push `toque` to its own upstream repo so it can be more easily shared across projects.

---
▸ 2026-09-27T19:44:49Z [Joel Webber]
All phases shorn 2026-09-27: toque split out and published (canon-c398), now playing/queue/transport + headless mode (canon-1522), browsing (canon-7b77), outputs/services/settings (canon-8b0b), docs and frames (canon-f0c6). Joel has used phase 1 daily.

---
▸ 2026-09-27T19:44:50Z [Joel Webber]
verify: `cargo test -p canon-tui` -> PASS (exit 0)
