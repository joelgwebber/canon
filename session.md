# Session log & next steps

Working notes, not a spec. `AGENTS.md` has the working agreements; `docs/arch.md` has the
architecture. This file is "where we left off".

Last updated at HEAD `893328b` (plus this doc pass).

---

## What we've done

Started from an empty repo and a yak herd, using the Python predecessor
(`/Users/joel/src/tideway`) as a reference for *what to do differently*. In rough order:

1. **Architecture pass** (`canon-dd79`) — settled the shape before writing much: one
   process, one state-owning actor, `Command` (intent) vs `EngineEvent` (reality),
   dependency direction inward on `canon-core`, one active output. Surveyed the Rust
   ecosystem for the thorny parts (decode, mDNS, Cast, TLS fingerprinting).
2. **Workspace + daemon skeleton** (`canon-16b7`) — seven crates, the `canon` binary,
   lifecycle and task supervision.
3. **Playback state core** (`canon-e284`) — `FrameClock`, the player actor, snapshot +
   `seq` + watch-based delta stream, device/sink changes as first-class transitions.
4. **Tidal source** (`canon-94cc`) — browser-impersonating HTTP (`wreq`), device-code
   *and* PKCE auth with rotating refresh, DASH/MPD → fMP4 segment resolution, the segment
   reader as a seekable `MediaInput` with 403 re-resolution. PKCE turned out to be
   required: device-code tokens no longer stream.
5. **Audio pipeline** (`canon-b192`, still shaving) — Symphonia decode, SPSC ring,
   realtime-safe cpal callback, in-track seek, the playback controller, server-owned
   queue with auto-advance.
6. **Control plane** (`canon-1190`) — axum WebSocket + JSON, and `canon control` as a
   line-oriented client so an end-to-end session is a single `printf | canon control`.
7. **Network sinks** (`canon-7718`, still shaving) — iface-aware self-healing mDNS
   discovery, LAN FLAC stream server with header replay, PCM→FLAC tap, the Chromecast
   sink, controller wiring with fail-back, and takeover detection via
   `consumers()`.
8. **Renderer position** (`canon-bc84`) — the fix that mattered: position on the network
   path comes from the *renderer's* reports read against the stream's origin, not from
   frames fed to the encoder.
9. **`AGENTS.md`** (`canon-5517`) and this doc pass (`canon-0292`).

**Verified on metal** against a KEF "Tunes" speaker: Tidal → decode → cpal, and Tidal →
FLAC → LAN HTTP → Chromecast, with seek/pause/resume/skip and Spotify-takeover fail-back.

Five bugs so far passed every unit test and failed instantly on a real speaker: a status
re-emit storm, a TTL reaping live devices, an invisible Spotify takeover, renderer
positions being stream-relative, and a stale dedup cache wedging playback. That's the
origin of the on-metal rule.

---

## Where the herd stands

57 yaks: 18 hairy, 2 shaving, 37 shorn (before this doc pass).

- **Shaving:** `canon-7718` (sinks) — one child left, `canon-685a` (DLNA).
  `canon-b192` (audio) — children `canon-390d`, `canon-4400`, `canon-caae`.
- **Inbox / needs human:** `canon-f04a` (settings schema) — parked at p3 on purpose,
  waiting on the first setting worth persisting. Two questions recorded on it: does the
  daemon want a config *file* for `bind`/`state_dir`, or are systemd flags fine; and
  ts-rs vs schemars for generated client types.

---

## Next: `canon-685a` — DLNA/UPnP control + GENA state feedback

Unblocked (its dependency `canon-bc84` is shorn) and the obvious next step. Parent
`canon-7718` is already shaving; shave `canon-685a` before writing code.

**The point of doing DLNA now** is that it's the adversarial second implementation. Cast
is a uniform spec with push status; DLNA is a device zoo with flaky eventing and coarse
position. If the sink abstraction survives it unchanged, it will survive our own remote
protocol later.

**It should inherit almost everything.** The test of a correct implementation is how
little it adds:

- `GetPositionInfo` / `RelTime` feeds the *existing* `EngineEvent::RendererPosition`.
  Reports are stream-relative; `RendererClock`'s `origin` already handles that. **Do not
  build a second position model.**
- Transport states map to the existing `EngineEvent::RendererState`. Conditions keep
  flowing; only edges dedup.
- Discovery, the LAN stream server, the FLAC tap, and the `consumers()` watchdog are
  reused unchanged.

**Specifics:**

- `rupnp` for SOAP AVTransport: `SetAVTransportURI` / `Play` / `Pause` / `Seek` /
  `GetTransportInfo`.
- GENA `LastChange` subscription for state feedback, with a `GetTransportInfo` poll as
  the fallback — device eventing is unreliable and some devices simply won't subscribe.
- `SetNextAVTransportURI` for gapless where supported (tideway never used it).
- **Version-agnostic service matching** — match on service type prefix, not an exact
  `:1`/`:2` string. Device-zoo hygiene.
- `RelTime` is truncated to whole seconds, which reads as a permanent ~0.5 s lag.
  `SLEW_BEHIND` (drift/8) was tuned for exactly this. Only revisit it with real evidence.

**Verify on metal.** "Tunes" (`192.168.0.205`) speaks *both* Chromecast and DLNA, so it's
a ready-made A/B: same speaker, same stream server, two control protocols. Exercise
transport, not just playback:

```sh
cargo build -p canon-daemon
(RUST_LOG=warn ./target/debug/canon serve > target/serve.log 2>&1 &)
sleep 18
printf 'sinks\nsink Tunes\nenqueue 33348478\nsleep 25\nseek +30\nsleep 10\npause\nsleep 3\nplay\nsleep 5\nquit\n' \
  | ./target/debug/canon control
pkill -f "canon serve"
```

Sinks select by name prefix. Discovery needs ~15 s before renderers appear. Note that
once DLNA lands, "Tunes" will be discoverable under two protocols — decide how that's
presented in `list_sinks` (one entry with a preferred protocol, or two) and record the
call on the yak.

---

## Then: ready work, roughly in order of value

- **`canon-4185` — library as source of truth**, with `canon-78ea` (entity model +
  sqlite), `canon-880e` (source bindings, ISRC/MBID resolution), `canon-5cb2` (local file
  index), `canon-65f7` (import/export). This is the *actual* differentiator — the reason
  canon exists rather than a Tidal client. `canon-library` is still a stub; the plan is
  written in its doc comment.
- **`canon-c67f` — MCP tool layer.** A second face over the same `Command` vocabulary,
  so an agent can drive playback and curate the library. Cheap once the library exists;
  much less interesting before it.
- **`canon-333e` — realtime bus + play reporting.** Note the trap recorded on the yak:
  its cross-device-pause half must arrive as an `EngineEvent`, **not** `Command::Pause`.
- **`canon-caae` — DSP chain** (ReplayGain, EQ, crossfeed, crossfade). Likely the trigger
  for `canon-f04a`, since it's the first thing with preferences worth persisting.
- **`canon-390d`** (proactive device-switch detection, sinc resampling) and
  **`canon-4400`** (cancel in-flight resolve on rapid skip/seek) — the last two audio
  children besides DSP.
- **`canon-0205` — multi-room.** The `OutputRoute`/`LocalGate` primitives are already
  built for it; this is where "one active output" gets revisited *with* the machinery.
- **`canon-1175` — Spotify feasibility.** Investigation only, no implementation.
- **`canon-f495`** — macOS ergonomics (firewall, Local Network privacy, a signed bundle).
  Parked; not a blocker, and irrelevant on the target Linux box.

---

## Standing reminders

- Never write code without an active shaving yak; shear only with pasted evidence.
- Commit the shorn yak move *with* the code it describes. Never push — there is no
  upstream.
- `edit_file` has corrupted large multiline inserts three times (`controller.rs`,
  `lib.rs`, `state.rs`). Use `write_file` for anything big.
- Background the daemon and `pkill -f "canon serve"` afterwards. No shell `$(...)` or
  `$VAR` in terminal calls; redirect to `target/*.log` and read the file.
- Test tracks: `33348478` (Björk, "Army of Me"), `520285418` (Dresden Dolls,
  "Backstabber").
