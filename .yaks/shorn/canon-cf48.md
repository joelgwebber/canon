---
id: canon-cf48
title: Sink trait + session lifecycle (RAII un-silence)
type: task
priority: 1
created: '2026-09-22T02:01:38Z'
updated: '2026-09-22T22:10:31Z'
parent: canon-7718
labels:
- sink
verify: cargo test -p canon-core
---

trait Sink { start(track); pause/resume/stop/seek; set_volume; health signal }. LocalSink, CastSink, DlnaSink. The player mutes local by ROUTING, not a global fill(0) flag that leaked forever in tideway. Un-silencing / route-restore is tied to dropping a session guard (Drop), so a skipped teardown is structurally impossible. Teardown is driven by a liveness detector (TCP close, Cast status timeout, DLNA poll), not only by a discovery-remove event.

---
▸ 2026-09-22T22:08:44Z [Joel Webber]
Scope for this pass (recorded so the fan-out siblings can build on it): deliver the load-bearing CONTRACT in canon-core only, fully unit-tested, no engine hot-path churn yet.
- Refine the Sink control-plane trait (network-renderer session): start/pause/resume/stop/seek/set_volume + id/kind.
- Liveness as an EVENT, not a poll: health() returns a watch::Receiver<SinkHealth>, so the player awaits a transition to Failed and fails back — teardown keys off liveness, never a discovery-remove.
- The RAII un-silence primitive: OutputRoute / LocalGate / RouteGuard. A single lock-free word (generation token, 0 = local owns output) that the local RT callback will consult; the ONLY way to restore local is dropping the guard, so no teardown/error/panic path can leave local silenced forever (tideway tide-4000.x).
DECISION: 'local' is modelled as the DEFAULT route (no network Sink holds it), not a fake LocalSink whose no-op pause/seek would lie. select_sink('local') = drop the route guard; select_sink(cast) = take it. Recording this because the original sketch listed a LocalSink type.
DEFERRED to canon-dde4 (where it's first exercised end-to-end): threading Arc<LocalGate> into AudioPlayer::start so the cpal callback emits silence when routed away. Landing that dormant now would be un-exercised hot-path code; the gate's own logic is fully tested here in isolation.

---
▸ 2026-09-22T22:10:21Z [Joel Webber]
verify: `cargo test -p canon-core` -> PASS (exit 0)

---
▸ 2026-09-22T22:10:31Z [Joel Webber]
Shorn. Landed in canon-core (crates/canon-core/src/sink.rs + lib.rs exports):
- Sink trait: network-renderer control plane (start/pause/resume/stop/seek/set_volume + id/kind), object-safe (Box<dyn Sink> tested).
- SinkHealth + liveness-as-event: health() -> watch::Receiver<SinkHealth>; player awaits Failed and fails back to local. No polling.
- RAII un-silence: OutputRoute / LocalGate / RouteGuard. Lock-free generation-token gate (0 = local owns output). take() mutes local; the guard's Drop (and ONLY that) restores it; a superseded guard's drop is a CAS no-op so switching renderers never flickers local audible.
Evidence: cargo test -p canon-core (10 pass incl. 6 route-invariant tests + object-safety/liveness); workspace fmt clean, clippy 0 warnings, all tests green.
Deferred (as scoped above): AudioPlayer cpal callback consulting Arc<LocalGate> lands in canon-dde4 where a real network route first exercises it.
