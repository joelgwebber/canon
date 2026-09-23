---
id: canon-44f4
title: 'Network transport never reaches the renderer: pause, stop, volume, mute'
type: bug
priority: 1
created: '2026-09-23T14:50:28Z'
updated: '2026-09-23T14:50:28Z'
parent: canon-7718
depends_on:
- canon-583a
labels:
- arch
- sink
---

From canon-ba30 (B), confirmed on metal 2026-09-23 against Tunes. The controller Play/Pause/Stop/SetVolume/SetMuted only touch AudioPlayer (the feed loop). CastSink::pause/play/stop/set_volume are never called.

Observed (canon control: sink Tunes, enqueue 33348478, sleep 20, pause, sleep 15, play, vol 0.3, mute, unmute, stop):
  playing 0:15 -> paused 0:16 -> playing 0:16 -> 0:18 ... 0:31   (through the 15s pause)
  ... 0:46 -> stop -> idle 0:00 -> playing 0:00/--:-- (no track)
The device reported player_state Playing with current_time advancing on every poll through the pause and after stop, and no volume/mute command appears in the cast trace. The speaker audibly ignores pause; the player honestly follows the device (device is authority), so the user pause is overwritten within one poll.

Two defects:
1. Transport/volume intent must be sent to the active renderer (through the canon-583a seam, which is why this depends on it). Pausing the feed alone just lets the device play out its buffer, which the LS50 holds for 15s+.
2. After Stop, a renderer condition revives the player from Idle to Playing with no track. Stop must end the renderer session (or at least stop the device), and the player should ignore RendererState when it has no renderer clock (Idle, and no stream loaded).

Verify on metal with the command above plus a longer pause. Pause must hold at a frozen position, stop must silence the speaker and stay idle, and vol must change the speaker volume.
