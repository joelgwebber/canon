---
id: canon-f04a
title: Unified settings/config (single serde schema)
type: task
priority: 3
created: '2026-09-22T02:01:37Z'
updated: '2026-09-23T20:09:56Z'
labels:
- architecture
needs: human
verify: cargo test -p canon-core settings && cargo test -p canon-daemon settings && cargo test -p canon-api
---

One serde struct is THE settings schema: it is the persisted file, the API PUT body, and the source of generated client types (ts-rs/schemars). deny-unknown-fields so a stale client key is a 4xx, not tideway's silent 200-no-op. Side-effects-on-change dispatched explicitly per field.

---
▸ 2026-09-23T04:33:03Z [Joel Webber]
Hoisted out of canon-16b7 rather than done now, because canon has no settings yet. Everything configurable today is CLI-only: --state-dir / CANON_STATE_DIR, --bind, --quality. There is no settings file, no PUT body, and no client to generate types for.

This yak's value is a RULE -- one serde struct is the schema for the file, the API body, and the generated client types, with deny-unknown-fields so a stale client key is a 4xx instead of tideway's silent 200-no-op -- and a rule needs fields to govern. Doing it now means either an empty struct plus codegen machinery, or inventing speculative settings, and then shipping a settings API surface with nothing real behind it. That surface ossifies fast.

The honest counter-argument, recorded so it is not lost: a hosted Linux deployment genuinely would rather put bind and state_dir in a file than in systemd flags. That is real but narrow, and satisfying it separately would create exactly the second schema this yak exists to prevent. When f04a happens, those deployment knobs join the same struct.

Trigger: the first user-facing setting that has to persist. Most likely canon-caae (DSP chain -- ReplayGain mode, EQ, crossfeed, crossfade all need stored preferences), or canon-4185 / canon-5cb2 (library roots). Do it with whichever lands first, not ahead of it.

---
▸ 2026-09-23T04:33:09Z [Joel Webber]
Parked at p3 until there is a setting worth persisting (see the note). Two things to confirm when it comes up: (1) do you want a config FILE at all for the daemon-deployment knobs (bind, state_dir), or is systemd/flags fine for your hosted box? (2) generated client types -- ts-rs or schemars? That choice only matters once a real UI consumes them, so it can wait, but it shapes the struct's derives.

---
▸ 2026-09-23T20:06:47Z [Joel Webber]
Picked up for flow mode (canon-77f8), minimally: the first real setting is the per-output mode (flow | standard). The two open questions moved to canon-d441 (needs human), since neither blocks this. Scope: one Settings serde struct in canon-core with deny_unknown_fields; persisted as settings.json in state_dir (written atomically; a file that fails to parse stops the daemon rather than being silently replaced); API ops `settings` (get) and `set_settings` (a full replacement, validated by the same struct, so a stale key is an error reply); `canon control` `settings` and `mode <output> flow|standard`. Keyed by output id (SinkInfo.id, one per physical speaker). The default for a network output is flow.

---
▸ 2026-09-23T20:09:48Z [Joel Webber]
Done (minimal). canon-core::Settings/OutputSettings/OutputMode (flow default | standard), with deny_unknown_fields at every level, and a SettingsStore trait. canon-daemon::settings::FileSettings writes settings.json in state_dir atomically (tmp + rename; persist, then publish), and a file that does not parse stops the daemon naming the path. API ops `settings` and `set_settings` (whole replacement; a stale key is refused by the schema). canon control has `settings` and `mode <output> flow|standard` (read-modify-write, output named like `sink`). Open questions moved to canon-d441.
Tests: an_empty_document_is_the_defaults, unknown_fields_are_refused_at_every_level, a_per_output_mode_round_trips, settings_survive_a_restart, an_unreadable_file_stops_the_daemon_instead_of_being_replaced; the ws test covers set/get and {"crossfade": 3} -> error "unknown field".
On the real daemon: `mode Tunes standard` -> settings.json {"outputs": {"LS50-Wireless-II-...": {"mode": "standard"}}}; restart the daemon; `settings` shows it unchanged; `mode Tunes flow` sets it back.

---
▸ 2026-09-23T20:09:56Z [Joel Webber]
verify: `cargo test -p canon-core settings && cargo test -p canon-daemon settings && cargo test -p canon-api` -> PASS (exit 0)
