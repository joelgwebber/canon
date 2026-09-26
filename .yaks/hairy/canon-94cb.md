---
id: canon-94cb
title: Fuzzy title+artist+duration matching as the persisted fallback
type: task
priority: 2
created: '2026-09-26T21:53:28Z'
updated: '2026-09-26T21:53:28Z'
parent: canon-880e
labels:
- library
---

When no ISRC matches onto a preferred service, search it by title+artist and bind a candidate whose normalized title and artist agree and whose duration is within a few seconds (provenance fuzzy, confidence from closeness). Persisted like any binding, so a re-source never re-fuzzes.
