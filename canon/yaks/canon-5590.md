---
id: canon-5590
title: Affordances for navigating through track details
status: hairy
type: idea
priority: 3
created: '2026-11-21T19:50:52Z'
updated: '2026-11-27T03:20:17Z'
parent: canon-a34e
tags:
- tui
---

Eg, by clicking or keyboard selection of artist(s), album, track, etc; to get to the library details.

The "library details" is ambiguous here, though. We need to think through what this really means in our design, which includes distinctions between local and remote libraries. Cases to consider:
- Jumping to, say, an artist should always start at the local library's details for that object. But what if there *is* no local version?
  - My instinct is that there always *should* be a local version, even if we have to fetch it from an upstream service and cache it on-demand.
- Within a local library object, there should still be some way to get to upstream services' versions. Maybe this is as simple as an affordance for copying a permalink from any we know about (eg, spotify vs tidal).
- This also suggests we *need* UI affordances for all the object types.
	- Pretty sure we have things like album, artist, track, et al in the data model, but I don't believe there's any explicit, canonical display/detail for these things in the TUI.
	- Are these "details" or top-level list views?

#### 2026-11-27T00:26:15Z

*herd-player*

Designed in [the third pass](/doks/design/tui-places.md) with Joel, 2026-11-21; built under its step yak (parent).

#### 2026-11-27T03:20:17Z

*herd-player*

Built by step 4 (canon-a34e): names in a detail are followed by Enter (history, o back), something not stored is fetched and stored without saving, y copies each service's link. Evidence on canon-a34e.
