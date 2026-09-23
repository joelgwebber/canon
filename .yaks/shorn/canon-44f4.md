---
id: canon-44f4
title: 'Network transport never reaches the renderer: pause, stop, volume, mute'
type: bug
priority: 1
created: '2026-09-23T14:50:28Z'
updated: '2026-09-23T16:13:26Z'
parent: canon-7718
depends_on:
- canon-583a
labels:
- arch
- sink
verify: cargo test --workspace
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

---
▸ 2026-09-23T16:04:21Z [Joel Webber]
Doing this together with canon-587a. Once Stop really reaches the speaker, the receiver reports Idle/Cancelled, which classifies as Ended. Tagged at receipt, that would auto-advance the queue after a user Stop. Design:
- Sink::load returns a LoadId, and the event stream carries RendererReport { load, event }. Cast tags each report with the load whose media session it describes, and resets its EdgeFilter per load.
- NetworkSession tracks current: Option<(LoadId, generation)>, set when a load is issued and cleared on Stop/Clear. Reports are forwarded only if their load is current and its generation is still the controller generation. Anything else describes media we have moved on from. This is a pure function, unit-tested for the race.
- The controller routes Play/Pause/Stop/SetVolume/SetMuted to the active sink as well as the feed loop. The feed must still pause, or a paused device would stop consuming and the broadcast ring would drop data on resume.
- Player: ignore RendererState/RendererPosition when no renderer stream is loaded (renderer clock None), so no report can revive Idle.
- Not changing: device volume is not read back into the snapshot yet, and selecting a sink does not push the player volume (1.0 would blast the speaker). Follow-up yak.

---
▸ 2026-09-23T16:13:24Z [Joel Webber]
Fixed, together with canon-587a. The controller routes Play/Pause to the loaded renderer (and still pauses the feed), Stop/Clear stop the renderer and forget its load, and SetVolume/SetMuted go to the renderer instead of the (ignored) feed gain. The player ignores RendererState/Position with no renderer stream loaded (unit test a_renderer_report_after_stop_is_not_news). The receiver volume reply is now logged at debug, since media status does not carry volume.

On-metal evidence, Tunes, canon control --json, snapshot runs collapsed by (seq, state, queue):
  sink Tunes / enqueue 33348478 / enqueue 520285418 / sleep 10 / vol 1 / sleep 2 / vol 0.3 / sleep 2 / pause / sleep 20 / play / sleep 5 / stop / sleep 10
  seq=4 playing queue=0/2 pos 0->8477
  seq=5 playing (vol 1)   ; device: cast volume set level=Some(0.01) muted=Some(false)
  seq=6 playing (vol 0.3) ; device: cast volume set level=Some(0.003) muted=Some(false)
  seq=7 paused  queue=0/2 frames=35 pos 12506->12506   (20s pause holds; device reported Paused on every poll)
  seq=8 playing queue=0/2 pos 12506->16763
  seq=9 idle    queue=0/2 pos 0->0                     (held 10s after stop: no revival, no auto-advance to track 2)
  mute/unmute (earlier run): cast mute set level=Some(0.003) muted=Some(true), then muted=Some(false)
  serve.log at warn: no warnings.
Before the fix (canon-ba30 run): pause -> paused 0:16 -> playing 0:16 ... 0:31 through the pause; stop -> idle -> playing 0:00/--:--.

Note: control vol takes a PERCENTAGE. My first run sent vol 0.3, which is 0.3%, and Tunes is left at that level (0.003). Its original level is unknown, so I did not guess one.

Follow-ups not done here: the device volume is not read back into the snapshot, so a fresh session shows volume 1.0 whatever the speaker is at. Selecting a sink deliberately does not push player volume to it. Filed separately.

---
▸ 2026-09-23T16:13:26Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)
