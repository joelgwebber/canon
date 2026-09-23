---
id: canon-0292
title: Architecture overview doc (docs/arch.md) + session handoff
type: task
priority: 2
created: '2026-09-23T12:43:53Z'
updated: '2026-09-23T12:47:07Z'
labels:
- docs
verify: grep -ohE 'canon-[0-9a-f]{4}' docs/arch.md session.md | sort -u > target/cited.txt; find .yaks -name 'canon-*.md' -exec basename {} .md ';' | sort -u > target/known.txt; comm -23 target/cited.txt target/known.txt
---

A pickup document: the runtime shape, the crate map and dependency direction, the seams (Source/Sink/Command/EngineEvent), the two position authorities, the hard-won state rules, and what is built vs not. AGENTS.md holds the working agreements; this holds the system's shape. Plus ./session.md recording where this session left off and what comes next.

---
▸ 2026-09-23T12:46:59Z [Joel Webber]
Wrote docs/arch.md (the system's shape) and ./session.md (where this session left off). AGENTS.md keeps the working agreements; no overlap beyond the on-metal harness command, which is worth repeating.

arch.md is organised around the things a plausible-looking change would break: the headless-no-local-device constraint and what it decided; the crate map and inward dependency direction; the four seams; the two position authorities (with the stream-relative-reports trap spelled out); conditions-vs-edges, seq-marks-transitions, liveness-is-consumers; and what is built vs stubbed. Each tideway-derived rule is tagged (tideway tax) with the failure it prevents, so the reason survives the rule.

Grounded in the tree rather than memory -- crate table read from the workspace, constants (NETWORK_LEAD 2s, SNAP 1.5s, SLEW_AHEAD/4, SLEW_BEHIND/8, POSITION_TICK 250ms, STREAM_PATH) read from source, dep table read from the Cargo.tomls, CLI verbs read from main.rs.

---
▸ 2026-09-23T12:47:01Z [Joel Webber]
verify: `grep -ohE 'canon-[0-9a-f]{4}' docs/arch.md session.md | sort -u > target/cited.txt; find .yaks -name 'canon-*.md' -exec basename {} .md ';' | sort -u > target/known.txt; comm -23 target/cited.txt target/known.txt` -> PASS (exit 0)

---
▸ 2026-09-23T12:47:07Z [Joel Webber]
Evidence.

Cited-yak-id check (the stored verify) -- all 26 ids cited across both docs resolve to real yaks, empty diff:

    cited but not a real yak:
    (end)
    26

Crate table matches the workspace exactly, 7/7 (canon-api, canon-audio, canon-core, canon-daemon, canon-library, canon-sink, canon-tidal), and every source path referenced in prose exists.

Green bar, docs-only change but run anyway:
    cargo fmt --all --check     -> FMT OK
    cargo clippy --workspace --all-targets -> Finished, zero warnings
    cargo build --workspace     -> Finished
    cargo test --workspace      -> all targets ok; 90 passed, 0 failed, 4 ignored
