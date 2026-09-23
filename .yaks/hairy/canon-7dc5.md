---
id: canon-7dc5
title: Send cover art in Cast load metadata
type: task
priority: 3
created: '2026-09-23T21:09:25Z'
updated: '2026-09-23T21:09:25Z'
parent: canon-7718
labels:
- sink
---

Tracks from the library now carry artwork_url (Tidal covers, 640x640), and DLNA already puts it in DIDL-Lite (upnp:albumArtURI), but the Cast LOAD sends no images, so the Nest Hubs (Kitchen, Library display) show a blank card. rust_cast's MusicTrack metadata takes images: Vec<Image>. In flow mode the display shows the stream's first track (known limitation); standard mode gets it right per track.
