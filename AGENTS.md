# canon — working agreements

A headless music daemon: one self-contained Rust binary that owns the library and
metadata, talks to music services (Tidal first), plays to local audio and to LAN
renderers, and exposes a WebSocket control plane for remote UIs, TUIs, and agents.
Upstream services are data sources, not the source of truth.

It must run headless on a Linux box with **no local audio device at all**, playing
only to network renderers. That constraint has already decided several designs
below; don't reopen them without accounting for it.

## Crate map

| crate | what lives there |
| --- | --- |
| `canon-core` | Entities, playback state, `Command`/`EngineEvent`, the player actor, `Source`/`Sink` traits. Everything depends inward on this; it depends on nothing of ours. |
| `canon-tidal` | PKCE auth, token refresh, stream resolution. |
| `canon-audio` | Symphonia decode, the cpal local output, the network feed loop. |
| `canon-sink` | Discovery (mDNS + SSDP), the LAN stream server, FLAC encode, the Chromecast and DLNA sinks. |
| `canon-api` | axum WebSocket + JSON control plane. |
| `canon-daemon` | The `canon` binary: `serve`, `login`, `devices`, `control`, and the playback controller (queue, auto-advance, sink sessions). |

## The green bar

All four, before anything is considered done:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets   # zero warnings, not "few warnings"
cargo test --workspace
cargo build --workspace
```

## Yaks

`.yaks/` is a committed team-mode farm; run the `yaks` CLI directly.

- **Never write code without an active shaving yak.** `yaks shave <id>` first.
- **Shear only with evidence.** `yaks update <id> --verify '<cmd>'` → `yaks verify <id>`
  → `yaks shorn <id>`. Paste real output into a note; "looks right" is not evidence.
- Commit the shorn yak move **with** the code it describes. Yak ids belong in commit
  messages. Commit as you go.
- **Never push.** There is no upstream.
- `yaks doctor --strict` stays clean.
- Leave open questions with `yaks ask <id> --note "..."` rather than stalling.
  (`yaks note` is not a command; it's `yaks update --note`, one `--note` per call.)
- If shipped work turns out to be wrong, `yaks regrow` it. Don't paper over a
  falsified claim with a fresh yak.

## Verifying against real hardware

**This is the point most likely to be forgotten, and it is the one that matters.**
Five bugs so far have passed every unit test and failed instantly on a real speaker:
a status re-emit storm, a TTL reaping live devices, an invisible Spotify takeover,
renderer positions being stream-relative, and a stale dedup cache wedging playback.
Unit tests alone are not grounds for shearing anything that touches a device.

`canon control` is a line-oriented client: one command per line on stdin, so it
drives from a terminal or a pipe. That makes an end-to-end session a one-liner.

```sh
cargo build -p canon-daemon
(RUST_LOG=warn ./target/debug/canon serve > target/serve.log 2>&1 &)
sleep 18   # discovery needs ~15s before renderers appear

printf 'sinks\nsink Tunes\nenqueue 33348478\nsleep 25\nseek +30\nsleep 10\npause\nsleep 3\nplay\nsleep 5\nquit\n' \
  | ./target/debug/canon control

pkill -f "canon serve"
```

- `sleep <secs>` keeps printing snapshots while it waits — that is how you watch a
  property hold over time. `--json` gives one server frame per line for `jq`;
  `--quiet` drops the once-a-second position echo.
- Sinks are selected by **name prefix** (`sink Tunes`), which takes the speaker's
  preferred protocol. `sink Tunes@dlna` (or `@cast`) pins one protocol for an A/B test.
  Renderer ids are mDNS service names and UPnP UDNs, and are not typeable.
- `vol` takes a **percentage**: `vol 30`, not `vol 0.3`. (`0.3` means 0.3%.)
- Standing hardware: KEF **"Tunes"** (`192.168.0.205`, speaks both Chromecast and
  DLNA), plus Kitchen, Basement speaker, Library display. Test tracks: `33348478`
  (Björk, "Army of Me") and `520285418` (Dresden Dolls, "Backstabber").
- Other harnesses: `canon devices --secs 6` lists every discovered endpoint (one per
  protocol), `canon resolve <track>` shows what a Tidal track resolves to, and
  `canon play-file <path>` plays a local FLAC/M4A through the engine with no daemon.
- Useful traces: `RUST_LOG=canon_core::player=trace` for position reconciliation,
  `RUST_LOG=info,canon_sink::cast=trace` for the Cast channel.

**Exercise transport, not just playback.** Steady-state streaming looked perfect for
42 seconds while two seek-related bugs sat underneath it.

## Settled architecture

Don't re-litigate these without new evidence; each was paid for.

- **One active output.** Switching sinks restarts the track at the current position.
  Chosen because headless-with-no-local-device must work, which rules out "keep the
  local device open but muted".
- **Device status is authoritative.** What a renderer reports about itself enters the
  state machine as an `EngineEvent`, never as a `Command` — a command is *user
  intent*, and laundering device status through one leaves the player unable to tell
  "the speaker is playing" from "someone pressed play". A renderer's end-of-track
  takes the same path as a local one, so queue auto-advance is identical on both.
- **Conditions vs edges.** `Playing`/`Paused`/`Buffering` are conditions and must keep
  flowing; suppressing repeats upstream is how the player ends up believing something
  the device is not doing. Only one-shot edges (`Ended`, `Superseded`, `Failed`) are
  deduped. Whether a report is a *transition* is the player's call, because only the
  player knows its own state.
- **`seq` marks transitions only.** Clients read a new `seq` as "something happened".
  Position refreshes and routine reconciliation re-emit under the same `seq`.
- **Position has one authority per output** (`canon-core/src/position.rs`). Local
  output derives it from frames the realtime callback emitted. A network renderer
  reports its own, and those reports are **relative to the stream it was handed** —
  after a seek that stream starts at the seek point, so they must be read against the
  stream's origin. Frames fed to the encoder are never position on that path; they run
  seconds ahead.
- **Liveness is "are our bytes being consumed"** (`StreamBroadcaster::consumers()`),
  which is protocol-agnostic and catches takeovers the control channel never mentions.
- **Prefer the signal the OS or library already owns.** A blind TTL once reaped live
  devices because it second-guessed mDNS remove events.

## Pitfalls

- **`edit_file` corrupts large multiline inserts.** It has mangled `controller.rs`,
  `lib.rs`, and `state.rs`. Keep edits small; use `write_file` for anything big;
  re-read after a large one.
- **macOS blocks inbound TCP to unsigned binaries** via the Application Firewall (not
  TCC, and loopback is unaffected). `target/debug/canon` is allow-listed; `target/debug/deps/<test>-<hash>`
  changes identity every build, so LAN-facing integration tests fail there. Local
  network permission is also required for mDNS and SSDP, and it does not always inherit.
  A `python3` probe of the LAN sees *nothing*, not even SSDP chatter, while `target/debug/canon`
  sees every device. Spike network code inside `canon` (for example `canon devices`), not in
  a script.
- **`rust_cast` is blocking**, with one mutex over the TLS stream held across reads.
  A second thread commanding while another blocks in `receive()` deadlocks. One
  thread owns all Cast I/O and alternates draining commands with a status poll.
- **No shell `$(...)` or `$VAR`** in terminal calls. `timeout` plus a pipe swallows
  output — redirect to `target/*.log` and read the file.
- Background the daemon and `pkill -f "canon serve"` afterwards.

## Commits

- One logical change per commit; each commit passes the green bar on its own.
- Subject in imperative present, under ~70 chars.
- The body explains **why**, not what — the diff already says what.
- No AI attribution or `Co-Authored-By` trailers.
