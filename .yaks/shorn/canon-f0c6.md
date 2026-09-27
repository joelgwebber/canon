---
id: canon-f0c6
title: 'TUI phase 4: docs and doc frames'
type: task
priority: 3
created: '2026-09-27T17:42:31Z'
updated: '2026-09-27T19:44:48Z'
parent: canon-3db9
labels:
- tui
verify: cargo test -p canon-tui doc_frames -- --ignored && test -s docs/assets/tui-queue.svg
---

docs/tui.md with keys and colour SVG frames rendered by a docshots test (toque render_to_svg), like yaks.

---
▸ 2026-09-27T19:44:46Z [Joel Webber]
Done 2026-09-27: docs/tui.md (tabs, every key, headless scripting, regenerating) with six colour frames in docs/assets/tui-*.svg, rendered by the ignored test doc_frames through toque::buffer_to_svg and looked at rasterized (qlmanage). Looking found two faults, both fixed: toque's SVG ignored reverse video, so no cursor showed (toque 72ac14c, pushed; canon's lock updated); and reverse video over dim columns made grey blocks, in a real terminal too, so the selection is now one band (white on 256-colour 238). Also README.md for the repo, and arch.md §11 brought up to date.

---
▸ 2026-09-27T19:44:48Z [Joel Webber]
verify: `cargo test -p canon-tui doc_frames -- --ignored && test -s docs/assets/tui-queue.svg` -> PASS (exit 0)
