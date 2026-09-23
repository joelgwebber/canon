---
id: canon-ba30
title: Architecture / code review
type: task
priority: 1
created: '2026-09-23T13:59:18Z'
updated: '2026-09-23T14:51:31Z'
labels:
- arch
verify: yaks list --label arch
---

Review the design and architecture in docs/arch.md, then look over the code to find anything that could be simplified, de-duped, etc.
Let's also review the architecture from the perspective of some high-level use-cases:
- A headless server controlling and streaming to external sinks on behalf of local UI clients.
- Network streaming to (as yet nonexistent) remote network clients (eg, a custom mobile app).
- Merging library databases across a mixture of services and local files.
- Perhaps eventually multiple users, with multiple databases and upstream service connections.

We don't have to _pre-architect_ for all these use-cases. Just review the architecture and code for design details that might make any of these particularly difficult to refactor into.

---
▸ 2026-09-23T14:02:13Z [Joel Webber]
REVIEW 1/3 - the sink seam is aspirational, and DLNA will hit that first.

(A) The Sink trait has no implementations. CastSink has inherent methods that shadow it (id/kind/play/pause/stop/seek/set_volume/health) but does not impl Sink, and the controller hardwires Option<CastSession> + Arc<CastSink> + CastEvent. Sink::start(&TrackMeta) also does not match what a renderer really does (load(url, meta), and maybe load_next(url) for gapless), and it has no event stream. OutputRoute/LocalGate/RouteGuard are unused (they are reserved for canon-0205, which is fine, but lib docs describe them as live). Before canon-685a: reshape Sink to match what Cast actually does (load/play/pause/stop/set_volume + an event receiver + health), impl it for CastSink, and make the controller hold Box<dyn Sink> in a protocol-neutral NetworkSession. Then DLNA is just a second impl, which is the whole point of doing it now.

(B) Renderer transport is never commanded. Controller Play/Pause only toggle AudioPlayer (the feed loop); CastSink::pause/play/set_volume/stop are never called from the controller. Predicted consequence: on pause the speaker keeps playing its ~2-4s buffer and keeps reporting Playing every 500ms, so RendererState(Playing) overrides the user Paused (device is authority), then the buffer starves and it goes to Buffering, i.e. Loading. Volume/mute on a network sink does nothing: run_network deliberately ignores gain, and nothing forwards it to the device. The canon-bc84 evidence (pause 0:58 -> play 0:58 with a 3s sleep) does not obviously fit this reading. Needs an on-metal check with a longer pause (sleep 15) and a set_volume before this is trusted.

(C) Device event translation is Cast-specific glue in the controller (run_cast_events maps CastEvent to EngineEvent), and the edge-dedup (CastEvent::is_edge + last) lives inside cast.rs. DLNA would duplicate both. Hoist a protocol-neutral RendererEvent {State, Position, Ended, Superseded, Failed} plus the edge-dedup into canon-sink, so each protocol only classifies. Also, SinkHealth::Failed and CastEvent::Failed are both emitted for the same failure. Pick one.

(D) Cast Ended is tagged with the generation current at *receipt* (run_cast_events reads inner.generation). If a user Next bumps the generation just before the old media Finished status arrives, that Ended is attributed to the new track and auto-advance skips one extra. Device events should carry the generation of the LOAD they belong to (map media_session_id to generation when LOAD is accepted).

(E) One URL per session (/stream.flac) and one FlacTap per track means SetNextAVTransportURI (DLNA gapless) has nowhere to point. Gapless needs per-track stream paths (e.g. /stream/<generation>.flac) served concurrently. Cheap to do while shaping the stream server for DLNA; the header-replay contract is unchanged.

(F) Dual-protocol devices: SinkId is the mDNS service name, so Tunes-over-Cast and Tunes-over-DLNA will have unrelated ids. Grouping needs a device identity (UPnP UDN / Cast UUID / IP) separate from the protocol endpoint. That decides the list_sinks question canon-685a asks.

---
▸ 2026-09-23T14:02:21Z [Joel Webber]
REVIEW 2/3 - state ownership, sources, library.

(G) The queue lives outside the actor. PlaybackController owns queue/index/active behind a Mutex and merges a QueueView into a second snapshot stream (run_snapshot_publisher). Consequences: queue changes do not bump seq (enqueue while playing only nudges dirty), which breaks the contract that a new seq means something happened. There are two snapshot streams, and log_snapshots in main.rs logs the player one, without the queue. QueueView is only {len,index}, so no client can render the queue. Direction: move queue state into the actor (Command::Enqueue etc. become real transitions there), with the controller as pure effect executor (resolve/engine/sink). That also removes the Command-vs-player no-op comment in command.rs and the Transition::Yes-for-every-command blanket (Play while Playing currently bumps seq).

(H) The Source seam is bypassed. The controller holds Arc<TidalSession> and calls open_stream_at, picks tidal_id() off TrackRef, and fails any non-Tidal track. Source::resolve (the trait method) is a dead buffered fallback (fetch_all pulls the whole track into a Cursor) and has no start position, which is why the controller could not use it. Fix before the library: Source::open(source, quality, start) -> (ResolvedStream, start_ms), a registry HashMap<Service, Arc<dyn Source>> in the daemon, and a resolver that walks TrackRef.sources by policy. That is the seam canon-880e and canon-5cb2 need, and it is small now.

(I) Identity: API play_track/enqueue mint a fresh random EntityId per call, so the same Tidal track enqueued twice is two entities. Harmless now, but the library must become the only minter, with the API going through it. SourceRef is a closed enum in canon-core (and Local { path } is a path, while the library plan wants content-addressed identity for local files). A closed enum is OK for a handful of services. Just flag Local{path} as provisional so the wire format is not frozen around paths.

(J) Service accounts are one per service, process-wide: AppState.sessions is HashMap<Service, ...>, tokens live at <state_dir>/tidal.json, and the controller holds a single TidalSession. Multi-user would key sessions by (user, service) and give each zone/user its own controller. Nothing to do now, but avoid adding more process-global singletons.

---
▸ 2026-09-23T14:02:32Z [Joel Webber]
REVIEW 3/3 - use-case lens and cleanup.

USE CASES
1. Headless server driving external sinks for local UIs: the shape is right (one actor, Command/EngineEvent, consumers() liveness, renderer-authority position). The gaps are (A)(B)(G) above. Both are fixable without redesign.
2. Streaming to a future canon remote client (mobile app as a sink): good fit. It is just another Sink that pulls the LAN stream and reports RendererState/RendererPosition. RendererClock and consumers() already generalise. Needs (A)(C) so a third protocol costs one impl. Off-LAN it would need auth plus a stream URL that is not an ephemeral port on a LAN IP. The control plane has no auth at all, which is fine only while bind defaults to loopback. Note that the snapshot exposes TrackRef.sources (local paths) to every client.
3. Merging libraries across services and local files: TrackRef{id, meta, sources: Vec<SourceRef>} is the right shape. The blockers are (H) (the controller is Tidal-concrete) and (I) (identity minted at the API edge).
4. Multiple users / zones: PlayerHandle + PlaybackController is already the natural zone unit, and ControlPlane is a trait object, so a registry of zones is additive. Blockers are (J) and the Command vocabulary having no zone/user addressing. That is additive at the protocol envelope. No change now.

CLEANUP (low-risk, independent)
- Three decode paths: canon-audio decode.rs (the c4c3 spike decode()), output.rs play_blocking (the v1 driver), and engine.rs Decode. Collapse onto engine::Decode and keep the spike only as a test helper, or retire the play-file subcommand in favour of the engine.
- codec_hint duplicated (controller.rs, main.rs run_play, cast.rs). lan_ip duplicated (controller.rs, daemon cast.rs).
- LOCAL_SINK in player.rs duplicates SinkInfo::LOCAL.
- canon_core::Event is unused.
- Dev harness subcommands (play, cast, resolve, play-file) predate canon control. Cast especially duplicates controller wiring. Consider keeping resolve/devices and dropping play/cast once control covers them.
- Every seek re-resolves Tidal playbackinfo+manifest (open_stream_at). A per-track manifest cache (canon-4c55 said and manifest cache) would cut seek latency. It overlaps canon-4400.
- Stale doc comments: canon-audio lib.rs says the local sink is a canon_core::Sink (it is not), canon-api lib.rs says the queue lands later (it has), and canon-sink lib.rs describes OutputRoute as live.

SUGGESTED ORDER: (A)+(C) (sink seam reshape, as the first step of canon-685a), (B) verify on metal then fix, (D)(E) during DLNA, (G) queue-into-actor before canon-c67f (MCP will want queue contents), (H)(I) as the first step of canon-4185. Cleanup whenever.

---
▸ 2026-09-23T14:51:26Z [Joel Webber]
On-metal check of (B), 2026-09-23, Tunes (LS50 Wireless II), canon control: sink Tunes / enqueue 33348478 / sleep 20 / pause / sleep 15 / play / sleep 8 / vol 0.3 / sleep 6 / mute / sleep 5 / unmute / sleep 3 / stop.
  playing 0:15 -> paused 0:16 -> playing 0:16 -> 0:18 -> ... -> 0:31  (the whole 15s pause)
  ... playing 0:46 -> idle 0:00/--:-- -> playing 0:00/--:--  (after stop, no track)
The cast trace shows player_state Playing with current_time advancing on every poll (e.g. 39.1, 39.6, 40.2 ... 47.98) through pause and after stop, and no volume/mute message was ever sent. (B) is confirmed and worse than predicted: the stop half was not in the original finding. Filed as canon-44f4.

Follow-ups filed, all labelled arch:
  canon-583a (A+C) sink seam, p1, child of canon-7718; canon-685a now depends on it
  canon-44f4 (B) renderer transport bug, p1, depends on canon-583a
  canon-587a (D) generation-tagged renderer events
  canon-e920 (E) per-track stream paths for gapless
  canon-7c6f (F) device identity vs protocol endpoint; canon-685a depends on it
  canon-fdf3 (G) queue into the actor; canon-c67f depends on it
  canon-ba9d (H) usable Source seam, child of canon-4185; canon-880e depends on it
  canon-f7da (I) library mints EntityId, depends on canon-78ea
  canon-7f16 cleanup chore
(J) multi-user: no yak on purpose, recorded above as a constraint (no new process-global singletons). The seek re-resolve latency is noted on canon-4400. canon-bc84 annotated: its pause evidence line does not hold.

---
▸ 2026-09-23T14:51:26Z [Joel Webber]
verify: `yaks list --label arch` -> PASS (exit 0)
