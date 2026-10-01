---
id: canon-8ed0
title: 'Export: create a new upstream playlist from a local one'
type: task
priority: 3
created: '2026-10-01T01:21:55Z'
updated: '2026-10-01T01:21:55Z'
parent: canon-65f7
depends_on:
- canon-5b2b
labels:
- library
- api
---

export_playlist(id, service): creates a NEW playlist on the service from the local playlist's tracks (resolving each to that service's catalog), never updates or deletes an existing upstream playlist. Description stamped with something like 'Exported from canon, <date>' so repeat exports are easy to find and delete by hand. Returns a report of tracks that didn't resolve on that service. Re-export = nuke the old one upstream yourself and export again; canon does not delete upstream (an export log to support canon deleting its own copies later is deferred).
