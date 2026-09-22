---
id: canon-f04a
title: Unified settings/config (single serde schema)
type: task
priority: 2
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T02:01:37Z'
parent: canon-16b7
labels:
- architecture
---

One serde struct is THE settings schema: it is the persisted file, the API PUT body, and the source of generated client types (ts-rs/schemars). deny-unknown-fields so a stale client key is a 4xx, not tideway's silent 200-no-op. Side-effects-on-change dispatched explicitly per field.
