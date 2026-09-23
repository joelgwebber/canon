---
id: canon-fdf3
title: Move the queue into the player actor; seq on queue changes; expose queue contents
type: task
priority: 2
created: '2026-09-23T14:51:02Z'
updated: '2026-09-23T19:42:56Z'
labels:
- arch
- state
- api
verify: cargo test --workspace
---

From canon-ba30 (G). The queue (queue/index/active) lives in PlaybackController behind a Mutex and is merged into a second snapshot stream by run_snapshot_publisher. Consequences: queue changes do not bump seq, which breaks the contract that a new seq means something happened (enqueue while playing only nudges dirty). There are two snapshot streams, and main.rs log_snapshots logs the one without the queue. QueueView is only {len, index}, so no client (TUI, MCP, mobile) can render the queue. Direction: queue state becomes actor state, the queue Commands are real transitions there, and the controller is a pure effect executor (resolve, engine, sink) driven by what the actor decided. One snapshot stream. Also stop counting every command as a transition (Play while Playing currently bumps seq). Watch the generation/auto-advance race semantics; they must survive the move. Before canon-c67f: MCP tools will want queue contents.

---
▸ 2026-09-23T19:31:08Z [Joel Webber]
DESIGN (functional core, imperative shell):
- The actor owns the queue, the current index, and the playback GENERATION. Queue commands are real transitions there, and so is auto-advance: an Ended for the current generation advances inside the actor.
- The actor emits Effects, decisions already made, on a channel: Start { generation, track, position }, Halt { generation }, Pause, Resume, SetVolume, SetMuted. The controller becomes a pure effect executor (resolve, engine, sink) and reports reality back as EngineEvents TAGGED with the generation they belong to. The actor drops stale ones, so there is one staleness rule in one place, replacing the controller Inner.generation checks and on_engine_event.
- SinkFailed is session-level and never stale. The actor computes the resume position itself when it restarts on local (atomic, instead of the controller reading a snapshot first).
- Commands get a reply (oneshot): "no next track" and "nothing to seek" are rejected by the actor, which knows its state. A no-op command is Transition::No and bumps no seq.
- New EngineEvent::Described(meta): the resolved display metadata written back into the queue entry, so it is fetched once.
- Snapshot: queue is always QueueView { len, index, revision }. Contents come separately (PlayerHandle::queue(), ControlPlane::queue(), API op "queue", control "queue"), so the 4 Hz position snapshots stay small. revision tells a client when to refetch.
- One snapshot stream: the controller merge publisher, the dirty Notify and queue_view go away.
- SelectSink: the controller opens the session (effectful, may fail), then commands SelectSink. The actor then emits Start at the audible position.

---
▸ 2026-09-23T19:42:46Z [Joel Webber]
Done, as designed above. The actor owns the queue, index, generation and auto-advance, emits Effects, and drops engine events tagged with a stale generation. Commands get a verdict. No-op commands are not transitions. Described(meta) writes metadata back into the queue entry. QueueView { len, index, revision } is in every snapshot, with contents via PlayerHandle::queue / ControlPlane::queue / the API op "queue" / control "queue". The controller is now an effect executor: its queue, Inner.generation, on_engine_event, processor task, merge publisher and dirty Notify are gone, as is the single snapshot stream. Also fixed while there: a new local AudioPlayer started at full gain, so after any track change the local volume silently reset while the snapshot kept the old level. The executor now applies the player volume/mute to each new local engine.

Tests (canon-core 37): sink_failure_falls_back_to_local_and_resumes, enqueueing_while_playing_is_a_transition, the_end_of_a_track_advances_the_queue, an_end_from_a_playback_we_left_is_ignored (canon-587a, now in the actor), impossible_commands_are_refused, a_command_that_changes_nothing_is_not_a_transition, stop_halts_and_keeps_the_queue, resolved_metadata_is_written_back_into_the_queue. The canon-api ws test covers enqueue -> snapshot queue.len=1, op queue -> tracks, and next -> {ok:false, "no next track"}.

On metal, same script on all three outputs (enqueue 33348478 + 520285418, seek +30, pause, play, next, prev, seek near the end, sleep, stop, queue):
  CAST (Tunes): QUEUE index=0 [Army of Me, 520285418] (metadata pending) ... QUEUE index=1 [Army of Me, Backstabber] (written back); seek 35944; next -> (1, Backstabber); prev -> (0, Army of Me); seek 3:44 -> playing 223654->233525 -> auto-advance (1, Backstabber) 0->23317; stop -> idle. Pause flapped paused/playing once per edge in one run (not in a rerun): a pre-existing command-in-flight race with the 500ms poll, filed separately.
  DLNA (Tunes@dlna): same shape, pause held (paused frames=13 at 42325), auto-advance 223654->233328 -> Backstabber, stop -> idle, no warnings.
  LOCAL (Mac speakers): pause held at 36814, next/prev, seek 3:48 -> 227647->234000 -> auto-advance to Backstabber, stop -> idle, no warnings.

---
▸ 2026-09-23T19:42:56Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)
