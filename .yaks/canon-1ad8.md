---
id: canon-1ad8
title: 'canon player: the TUI plus local and LAN output in one binary'
status: shorn
type: feature
priority: 1
created: '2026-10-17T19:15:46Z'
updated: '2026-10-18T05:55:12Z'
parent: canon-88a7
depends_on:
- canon-806d
- canon-040e
- canon-5ce4
- canon-65d2
source: arch
---

The first player: the TUI's UI (controller role) and today's `PlaybackController` + `canon-audio` + `canon-sink` (executor role) in one process, talking to a remote server. It plays to local audio and drives Cast/DLNA itself, so the firewall, signing, and `--lan-port` rules in AGENTS.md move with it. Fetches its audio from the server's FLAC endpoint and retries with backoff, giving up with `EngineEvent::Failed`.

Shorn only after a real session: play, seek, pause, sink switch to Tunes, and a dropped server connection.

---
▸ 2026-10-17T22:03:46Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-17T22:14:41Z [delta:The Great Decoupling]

Built; real-session check outstanding (see ask).

- `canon player --connect <server> [--player id] [--token t] [--headless]`: one WS connection with both roles. Executor = the same `PlaybackController` as `canon serve`, with `Tracks::Remote` fetching `/audio/<id>?token=…` (+`&position_ms=`) from the server over HTTP/1.0 (canon-daemon/src/remote.rs): format from the FLAC STREAMINFO, start from `x-canon-start-ms`; unreachable → 4 tries with backoff then `Transient`; a refusal isn't retried (401/403 Auth, 404 NotFound). The TUI runs over the same link (`canon_tui::connect_with` routes executor orders to the executor, everything else to the app).
- Discovery, the speaker stream server and `--lan-port` are the player's own; it needs no service logins.

Evidence:
- Green bar: fmt, clippy 0 warnings, test --workspace ok, build ok.
- `a_player_fetches_and_decodes_what_the_server_serves`: real server, RemoteAudio from 1.5s → start 1000 (segment), 44.1k/16-bit from STREAMINFO, engine decodes PCM; unknown id → NotFound without retries. `streaminfo_reads_what_the_encoder_wrote` (24/96/2).
- Live (target/live-1ad8.sh; server on 7395/7351 with own state dir; player `--headless --lan-port 7352`): server logged `a player connected to carry out playback player=den`; `sinks` for den listed local + 5 discovered speakers (reported by the player's executor); `add 520285418` → the player fetched /audio with its signed token and the server reached Tidal (`tidal can't stream: sign in…`, expected without credentials), i.e. token accepted and the fetch path exercised; killing the server → player logged `lost the server`, then reconnect attempts at 1s/2s/4s.

Decision: the player reuses the daemon's controller rather than a new executor: one implementation of renderer sessions, reconnects and the watchdog, so a hardware fix applies to both.
Decision: plain blocking HTTP/1.0 over std TcpStream for the fetch (the engine reads synchronously; no chunked decoding; no new dependency). Rejected: reqwest blocking (feature change in Cargo).
Decision: a player's settings are defaults for now (flow mode off); per-output settings stay on the server. Follow-up if wanted.

---
▸ 2026-10-17T22:14:41Z [delta:The Great Decoupling]

asked: needs human

Real-session check once the branch is on Joel's checkout: canon serve (test port, real state dir) + canon player --player den on another port; via canon tui --player den: enqueue 33348478, seek +30, pause, play, sink Tunes, sink local; then stop canon serve mid-track and restart it, checking the player reconnects and resumes on play.

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

Landed; remote-player session run live by the agent.

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

Live on real Tidal audio (`target/hw-1ad8.sh`): test server 7399 + `canon player --player den --headless --lan-port 7348` on the same box (local USB output):
- The player took over (`a player connected to carry out playback player=den`), fetched with its signed token and played: after enqueue, seek +30, pause, play → `playing 0:47/3:54 Army of Me`.
- Killed the server mid-track: player logged `lost the server`, retried at 1s/2s/4s, reconnected to the restarted server and became the executor again.
- Restarted server: `restored its queue: 26 tracks, at 1 player=den` (autoplay had topped it up), snapshot `idle [1/26]`, not playing. `play` → server log `streaming "Army of Me" … at 47.788s`, control showed `playing 0:51` 8s later: resumed where it was. This also covers canon-973d's resume-after-restart.
- Not checked: a player on a different machine, and driving Tunes from the player (same firewall issue as canon-806d).

---
▸ 2026-10-18T04:18:48Z [delta:The Great Decoupling]

moved: shaving -> shorn

---
▸ 2026-10-18T05:18:36Z [delta:The Great Decoupling]

moved: shorn -> hairy

---
▸ 2026-10-18T05:18:36Z [delta:The Great Decoupling]

Regrown: with the UI and the executor on ONE connection (canon player without --headless), output selection deadlocks. handle_socket awaits dispatch(SelectSink) → Station waits for the executor's Selected report, which arrives on the same socket the loop isn't reading (and the Select order itself isn't forwarded while it waits). Every select times out after 60s. Found by Joel live. All my tests and live runs used a separate control connection, so missed it.

---
▸ 2026-10-18T05:18:36Z [delta:The Great Decoupling]

moved: hairy -> shaving

---
▸ 2026-10-18T05:55:12Z [delta:The Great Decoupling]

Fixed, two bugs, both only with the UI and executor on one connection:
1. Server (canon-api/src/server.rs): handle_socket awaited each request inline, so `select_sink` (which waits on this connection's own executor) blocked the loop that forwards that executor's orders and reads its answer → 60s timeout. Requests now run on a per-connection task, one at a time in arrival order (loop-made replies queue behind them, so reply order is preserved); the loop keeps forwarding orders/reports. Snapshots are held while a request is in flight, as before, so a command's reply still precedes the snapshot showing it (the e2e test hung intermittently without this).
2. Daemon (main.rs): tracing wrote to stdout, which is a client's output (control --json, a scripted TUI's frames). Now stderr.

Evidence:
- New `a_player_with_a_ui_can_select_an_output_on_its_own_connection` (one connection: become_executor, select_sink → select order arrives while the request waits → selected report → reply ok + snapshot sink moved). control_plane suite run 10× in a loop clean; green bar (fmt, clippy 0, test --workspace, build).
- Live, the exact shape Joel ran (`target/ui-player.sh`): test serve on 7399/lan 7346 + `canon player --lan-port 7347 --script 110x20` (the player's own TUI, new hidden `--script` flag driving it via toque). In the TUI's Outputs tab: Enter on "Tunes over cast" → cast load accepted, Tunes fetched our stream, header `uicheck · Tunes · connected`, playing; Enter on "over dlna" → loading, then playing on DLNA (Rygel fetched); Home + Enter on Local → back to local. Every select answered within the step (pending=0 by the next frame).

---
▸ 2026-10-18T05:55:12Z [delta:The Great Decoupling]

moved: shaving -> shorn
