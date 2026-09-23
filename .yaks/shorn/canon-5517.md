---
id: canon-5517
title: 'Project AGENTS.md: working agreements, invariants, on-metal harness'
type: task
priority: 2
created: '2026-09-23T04:09:27Z'
updated: '2026-09-23T04:10:38Z'
labels:
- docs
verify: ./target/debug/canon control --help | grep -q -- --json && ./target/debug/canon control --help | grep -q -- --quiet && ./target/debug/canon devices --help | grep -q -- --secs && ./target/debug/canon cast --help | grep -q -- --to
---

The repo has no local agent instructions, so every session rediscovers the same things: the yaks discipline, the green-bar commands, which architectural decisions are settled, and -- the expensive one -- that on-metal verification is now a one-liner through 'canon control'.

Five bugs so far have been invisible to unit tests and visible only against real hardware (status re-emit storm, TTL reaping live devices, Spotify takeover, renderer reports being stream-relative, the state dedup wedging playback). An agent that does not know the harness exists will not use it, and will shear yaks on unit tests alone.

---
▸ 2026-09-23T04:10:29Z [Joel Webber]
verify: `./target/debug/canon control --help | grep -q -- --json && ./target/debug/canon control --help | grep -q -- --quiet && ./target/debug/canon devices --help | grep -q -- --secs && ./target/debug/canon cast --help | grep -q -- --to` -> PASS (exit 0)

---
▸ 2026-09-23T04:10:38Z [Joel Webber]
DONE. AGENTS.md written at the repo root: crate map, the four-command green bar, the yaks discipline, the on-metal harness, the settled architecture, and the pitfalls that have actually cost time (edit_file mangling large inserts, the macOS Application Firewall vs unsigned test binaries, rust_cast's blocking single-mutex client, no shell substitution in tool calls).

The section that earns its place is 'Verifying against real hardware'. It leads with the count -- five bugs so far that passed every unit test and failed instantly on a speaker -- then gives the canonical pipeline as a copyable one-liner, the standing hardware and test track ids, the ~15s discovery wait, and the RUST_LOG targets. It closes on 'exercise transport, not just playback', which is the lesson from two seek bugs hiding under 42 seconds of flawless streaming.

'Settled architecture' is written as things not to re-litigate without new evidence, each with the reason it was paid for, so a future session does not rediscover why device status is an EngineEvent rather than a Command, or why conditions must keep flowing while edges dedup.

verify: checks the documented CLI surface still exists (control --json/--quiet, devices --secs, cast --to), which is the part of a doc most likely to rot silently.
