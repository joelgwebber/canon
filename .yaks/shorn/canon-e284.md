---
id: canon-e284
title: Playback state core — single source of truth
type: feature
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-23T04:32:33Z'
labels:
- state
verify: cargo test -p canon-core --lib
---

The architectural centerpiece. Fixes the class of bug behind tide-2f85: reality (the audio device / active sink) moving without the emitted state moving. Everything that can change playback reality must flow through one state machine that re-emits.

---
▸ 2026-09-23T04:32:24Z [Joel Webber]
verify: `cargo test -p canon-core --lib` -> PASS (exit 0)

---
▸ 2026-09-23T04:32:33Z [Joel Webber]
All four children shorn: 5afb (player actor + event bus), e5e8 (FrameClock: atomic frames + device epoch), 9487 (snapshot/seq stream), 08a9 (device and sink changes as first-class transitions).

The parent's claim -- everything that can change playback reality flows through one state machine that re-emits, so the published view cannot silently desync (the tide-2f85 class) -- held up under pressure it was not originally designed against, and got sharper for it:
- Position grew a second authority (canon-bc84) without a second state machine: PositionDrive picks which clock owns position per stream, and the renderer's reports enter as ordinary EngineEvents.
- The Command/EngineEvent split earned itself twice. Device reports arrive as engine events, never as commands, so the player can tell 'the speaker is playing' from 'someone pressed play'.
- The seq contract survived continuous reconciliation at ~2/sec: handle() now returns Transition, so only a real change bumps seq, and routine corrections re-emit under the same one.

Verified live throughout: playback to local output and to a Chromecast renderer, with seek, pause/resume, queue auto-advance, external-takeover fail-back, and position tracking a real device within ~200ms.
