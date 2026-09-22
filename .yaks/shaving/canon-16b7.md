---
id: canon-16b7
title: Workspace, daemon skeleton & config
type: feature
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T02:53:39Z'
parent: canon-dd79
labels:
- architecture
---

The cargo workspace, the headless daemon process, its async runtime (tokio), structured logging, and lifecycle. Deliberately small crates with clear seams so no single file becomes the tideway monolith. This is the scaffold B-G hang off.
