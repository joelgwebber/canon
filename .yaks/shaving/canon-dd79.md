---
id: canon-dd79
title: 'Canon core: headless music daemon — architecture & plan'
type: feature
priority: 1
created: '2026-09-22T02:01:37Z'
updated: '2026-09-22T03:40:53Z'
labels:
- architecture
---

Canon is one self-contained Rust binary: a headless daemon that talks to music services (Tidal first), system audio devices, and LAN renderers (Chromecast/DLNA/OpenHome), owns the library + metadata, and exposes a control API + MCP tools so UIs and agents drive it remotely. Upstream services are DATA SOURCES, not the source of truth.

Seven prime directives, each distilled from a concrete tideway failure (tide- ids reference tideway's own tracker):
1. ONE authoritative playback state. tideway's state emitter desynced from reality after device-loss+reconnect (tide-2f85) because position/liveness lived in the realtime callback while transport state lived elsewhere and the device could change under both. Canon: a control actor owns an explicit state machine; the RT callback only publishes atomic counters; device/sink changes are first-class transitions; clients get snapshot+seq+delta.
2. Sinks are a trait with RAII lifecycle. tideway leaked casting sessions so local output stayed silenced and the player got stuck unresumable (tide-4000.2/.3). Canon: Local/Cast/Dlna are polymorphic sinks; un-silencing is tied to dropping a session guard so teardown cannot leak.
3. Discovery is a supervised, self-healing service. tideway's discovery wedged on VPN/utun multicast capture, sleep/wake IGMP drops, and :1900 contention with no recovery (tide-6fd0, tide-8f5b). Canon: enumerate real LAN interfaces excluding tunnels, pin multicast egress, rebuild sockets + rejoin groups on wake, watchdog wedged reception.
4. Sources re-resolve transparently. Tidal's signed segment URLs expire ~3 min and tideway just crashed/dropped the track (tide-1100, tide-bd9e). Canon: a 403 on a segment re-resolves the manifest and resumes at the current index; decode treats fetch errors as recoverable.
5. Realtime discipline. The output callback is allocation/lock/syscall-free. No GIL means we can run real low-latency buffers instead of tideway's 100ms floor. OS UI/audio-control frameworks are kept on their required threads.
6. One schema per concept. tideway's settings silently no-op'd because a field existed in the dataclass but not its Pydantic mirror. Canon: a single serde struct is API body + persistence + generated client types, deny-unknown-fields.
7. Modularity. Replace the 4.5k-line player + 14k-line server monolith with a workspace of focused crates.

Proposed crate map: canon-core (entities, state machine, traits) | canon-daemon (bin) | canon-tidal (source) | canon-audio (decode/dsp/output) | canon-sink (local + network + discovery) | canon-library (persistence) | canon-api (control + MCP). Children A-G are the workstreams; B/C/D/E are the critical path for 'Tidal -> internal/external devices'.

---
▸ 2026-09-22T03:40:53Z [Joel Webber]
test
