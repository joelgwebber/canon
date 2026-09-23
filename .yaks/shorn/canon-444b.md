---
id: canon-444b
title: Scriptable control-plane client for on-metal verification
type: task
priority: 2
created: '2026-09-23T03:49:23Z'
updated: '2026-09-23T04:03:23Z'
labels:
- api
verify: cargo test -p canon-daemon --bin canon control
---

Verifying canon-bc84 on real hardware needed a client that could list sinks, select one, enqueue a track and print snapshots non-interactively. 'canon control' is a raw-mode keypress TUI, so it cannot be driven from a script or by an agent, and the gap was filled with a throwaway Python websockets script under target/ (gitignored, now gone).

Every future network-sink yak needs the same harness -- canon-685a (DLNA) most immediately, since proving a renderer's position feedback works means watching published snapshots against device reports over a real session.

Shape: non-interactive ops on the existing ws control plane, e.g. 'canon ctl list-sinks', 'canon ctl select-sink Tunes', 'canon ctl enqueue tidal:520285418', 'canon ctl watch --json'. A line-oriented --json mode makes it pipeable, and is also the natural thing for an agent to drive, which is one of canon's stated goals anyway.

---
▸ 2026-09-23T03:49:37Z [Joel Webber]
Worth building this as a first-class 'canon ctl' subcommand, or should it stay a dev script? Leaning first-class: agents driving canon over the control plane is a headline goal, and a line-oriented JSON client is most of what an MCP shim would need anyway -- so this may really be an early slice of the MCP surface rather than test tooling. Priority 4 for now; say the word and it moves up.

---
▸ 2026-09-23T03:54:26Z [Joel Webber]
Why don't we just change `canon control` to be a very simple, non-raw CLI, with basic commands? We can make a "real" TUI later.

---
▸ 2026-09-23T03:56:20Z [Joel Webber]
Answered: no separate 'canon ctl'. Turn 'canon control' itself into a plain, non-raw line-oriented CLI with basic commands; a real TUI can come later on top of the same control plane.

---
▸ 2026-09-23T04:03:07Z [Joel Webber]
verify: `cargo test -p canon-daemon --bin canon control` -> PASS (exit 0)

---
▸ 2026-09-23T04:03:15Z [Joel Webber]
DONE, per the answer: 'canon control' is now a plain line-oriented client rather than a raw-mode keypress TUI, so one client serves a human at a terminal and a script or agent through a pipe. Commands: play/pause/stop/next/prev/clear, seek (90 | 1:30 | +10 | -10), vol (60 | +10) / mute / unmute, enqueue, sinks, sink <name-or-id>, status, sleep <secs>, help, quit. --json prints every server frame as one JSON line for jq; --quiet drops the once-a-second position echo, which otherwise prints only while playing so an idle client stays silent. Selecting a sink by name prefix matters more than it looks: renderer ids are mDNS service names, unmemorable and unguessable, so 'sink Tunes' is the only usable form from a script. Dropped the crossterm dependency entirely.

On metal, end to end through a pipe against the KEF:
  printf 'sinks\nsink Tunes\nenqueue 33348478\nsleep 25\nseek +30\n...' | canon control
listed 5 discovered sinks, selected Tunes by name, played Bjork on it, and echoed position once a second.
