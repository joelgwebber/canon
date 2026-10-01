---
id: canon-b3dc
title: Live-test daemons against the real state dir can wedge it when parallel lanes add schema migrations
type: task
priority: 2
created: '2026-10-01T02:25:54Z'
updated: '2026-10-01T02:25:54Z'
labels:
- library
- infra
---

Root cause of the canon-65f7 library-wedge incident (2026-09-30): canon-120d's and canon-5b2b's parallel lanes each ran a live canon control smoke test on their own port, but AGENTS.md's convention has test daemons share the real state dir (including library.sqlite) unless --state-dir is passed. canon-120d's build ran its own not-yet-renumbered migration against the real db, bumping PRAGMA user_version to what was then its local index. When the coordinator merged three lanes that each independently appended a schema migration (canon-ae6e, canon-120d) and renumbered them to resolve the conflict, the real db's counter no longer matched its own content: it had migration 9's tables but not migration 8's, and user_version said 8. canon serve failed to start until the gap was hand-repaired (with Joel's explicit approval, after a backup).

This will recur with any future parallel batch where more than one lane adds a schema migration and at least one lane does a live check against the shared state dir before all lanes land. Fix one or both of:
1. Make migrations idempotent/order-independent (CREATE TABLE IF NOT EXISTS, or claim migration indices up front across a batch instead of at merge time), so a renumbering at merge can never desync a database that already ran an old numbering live.
2. Tighten the AGENTS.md live-verification convention: a lane whose change touches schema.rs should use its own --state-dir for live checks (never the shared real one) until it has landed on main, specifically because 'own port' does not imply 'own database file.'

Add a line to AGENTS.md's Pitfalls section once the fix is picked.
