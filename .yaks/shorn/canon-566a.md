---
id: canon-566a
title: Current volume not always updated for output device
type: bug
priority: 1
created: '2026-10-01T12:19:52Z'
updated: '2026-10-01T12:41:47Z'
verify: cargo test --workspace
---

When I connect to a cast device, the initial volume shows as 100% (narrator: it's not). When I go to adjust the volume down from there by 5%, it immediately jumps to 95%, scaring the ever-loving shit out of my cat.

We should figure out what's necessary to keep the volume up to date against any output device. If there's a cost/latency tradeoff in keeping it updated, we should at least ensure the interface for _adjusting_ the volume starts by getting the correct current value before updating it. My cat thanks you in advance.

---
▸ 2026-10-01T12:41:38Z [Joel Webber]
Root cause: canon never learned a renderers real volume at all. The Sink trait is fire-and-forget commands only (set_volume, no get), player state defaulted volume to 1.0 and only ever changed it from user Command::SetVolume -- so it started every connection assuming full volume regardless of reality, and a relative vol -5 computed from that wrong cached value, not the speakers actual level. Hence 100% on connect and a scary jump on first adjust.

Fix, keeping the existing fire-and-forget Sink architecture (device status is authoritative, reports enter as EngineEvents never Commands):
- New canon-core RendererEvent::Volume{level,muted} + EngineEvent::RendererVolume + PlayerHandle::renderer_volume(), not scoped to any load (a renderer keeps its volume across loads) -- same Input::Engine(None, ...) path as sink_failed. Updates self.volume/self.muted and transitions (so clients see it) without re-commanding the sink -- a report is not a command.
- Cast (canon-sink/cast.rs): reads device.receiver.get_status() once right after connecting, before the main loop, and reports it. SetVolume/SetMuted already get the receivers resulting Volume back for free (rust_cast); that was previously only logged at debug and is now forwarded as a report too, so a clamp/step the device applies is caught.
- DLNA (canon-sink/dlna.rs): added Renderer::rendering_query (GetVolume/GetMute) alongside the existing fire-and-forget rendering (Set*), read once at connect and once after any Set (DLNA does not echo the result the way Cast does, so confirming it costs a deliberate follow-up GetVolume/GetMute -- not added to the continuous 500ms transport poll, to avoid doubling that protocols chatter for a value that only changes when asked).
- Deliberately NOT implemented: continuous background repolling of volume to catch changes made by another controller (e.g. the Google Home app) while canon is not the one setting it. That is a real gap but a different, continuous-cost problem from the one reported; the ask here (100% shown is wrong, and adjust must start from the right value) is fully covered by connect-time + post-set reporting.

RendererEvent/RendererReport dropped their Eq derive (kept PartialEq) since Option<f32> cannot be Eq; grepped for any Eq-bound usage first, found none.

Unit tests: canon-core/player.rs (renderer_volume updates snapshot without emitting Effect::SetVolume/SetMuted -- a report is not a command; partial level-only/mute-only reports; a matching report is not a transition) and canon-sink/dlna.rs (current_volume/current_mute UPnP parsing, including the >100 boost-range clamp). dispatch()/command_once() themselves are not unit tested, consistent with the rest of these two files -- the real protocol I/O is live-verified below, not mocked.

Live-verified against the real Tunes KEF speaker on both protocols:
- sink Tunes@cast: snapshot volume corrected from the 1.0 default to the speakers real 0.25 within ~1s of connecting. vol -5 then computed 0.25 -> 0.20 (not 1.0 -> 0.95) -- confirmed via --json snapshot stream.
- sink Tunes@dlna: reconnecting (same physical speaker) read back 0.20 -- consistent with where Cast had just left it, cross-protocol. vol +10 computed 0.20 -> 0.30, confirmed via the post-set GetVolume readback.

Full green bar passes: fmt --check, clippy --workspace --all-targets (0 warnings), test --workspace, build --workspace.

---
▸ 2026-10-01T12:41:47Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)
