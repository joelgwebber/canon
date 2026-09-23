---
id: canon-444b
title: Scriptable control-plane client for on-metal verification
type: task
priority: 4
created: '2026-09-23T03:49:23Z'
updated: '2026-09-23T03:49:37Z'
labels:
- api
needs: human
---

Verifying canon-bc84 on real hardware needed a client that could list sinks, select one, enqueue a track and print snapshots non-interactively. 'canon control' is a raw-mode keypress TUI, so it cannot be driven from a script or by an agent, and the gap was filled with a throwaway Python websockets script under target/ (gitignored, now gone).

Every future network-sink yak needs the same harness -- canon-685a (DLNA) most immediately, since proving a renderer's position feedback works means watching published snapshots against device reports over a real session.

Shape: non-interactive ops on the existing ws control plane, e.g. 'canon ctl list-sinks', 'canon ctl select-sink Tunes', 'canon ctl enqueue tidal:520285418', 'canon ctl watch --json'. A line-oriented --json mode makes it pipeable, and is also the natural thing for an agent to drive, which is one of canon's stated goals anyway.

---
▸ 2026-09-23T03:49:37Z [Joel Webber]
Worth building this as a first-class 'canon ctl' subcommand, or should it stay a dev script? Leaning first-class: agents driving canon over the control plane is a headline goal, and a line-oriented JSON client is most of what an MCP shim would need anyway -- so this may really be an early slice of the MCP surface rather than test tooling. Priority 4 for now; say the word and it moves up.
