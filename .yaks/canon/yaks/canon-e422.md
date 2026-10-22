---
id: canon-e422
title: 'Settle canon into server and clients: commands, options, cruft'
status: hairy
type: task
priority: 2
created: '2026-10-18T19:09:56Z'
updated: '2026-10-18T19:09:56Z'
parent: canon-88a7
source: arch
---

Decide, then do, the shape canon settles into now that playback is decoupled: **one server, and clients**. Supersedes the "flag or default" question in canon-f052: the server never produces audio.

## Roles

- **Server (`canon serve`)**: library, service logins, settings, one actor+queue per player, autoplay, `/audio`, pairing. No discovery, no LAN stream server, no built-in executor, no `--lan-port`. A player with no client attached fails a start at once (the Station's no-executor path).
- **Client**: UI and audio. Connects to a server, becomes its player's executor, plays to this machine's output or this LAN's speakers. `canon tui` is the first one.

## Proposals (to confirm with Joel)

1. **Merge `tui` and `player`** into one client command. It plays by default (takes over its player's executor, as `player` does now); `--remote` controls without playing (today's `tui`, for driving a player that plays elsewhere). The toque driver becomes `--script WxH [--diff]` (today `tui --headless` and `player --script` mean the same thing under different names). Keep a no-UI mode (`--headless`): it is how the founding constraint (a Linux box, no audio device, LAN speakers) is met now: `serve` + a headless client on the same box.
2. **Retire `canon login`.** Every service login (Tidal PKCE, Spotify web, librespot) already works over the protocol from the TUI's Settings tab; the CLI only does Tidal, in two manual steps. `pair` stays: it authorizes a *client* to the server, which is a different thing (server-side admin, run on the server's machine).
3. **`canon debug <…>`** for the diagnostics: `control`, `resolve`, `tidal-get`, `play-file`, `devices`. One line in top-level help. Retire `spotify-play` (the canon-52fb spike) once librespot is verified through `/audio`, which is the real path now.
4. **Options belong to the command that uses them.** `--bind`, `--state-dir` on `serve` (and the debug commands that read logins). `--connect`, `--player`, `--token`, `--lan-port` on the client (and `debug devices`). The client gets its own state dir for its logs (later: a saved token per server), so a client never opens the server's library.
5. Bare `canon` prints help instead of starting a server.

## Fallout the server losing its executor forces (not optional)

- **Per-output settings reach the client.** `canon player` runs on `Defaults` (flow mode off everywhere); per-speaker settings live on the server. Once the server's controller is gone, that's a regression for everyone. The client reads (and follows) the server's settings over the protocol.
- **Renderer metadata.** `Tracks::Remote` returns no `track_meta`, so Cast/DLNA show no title or art when a client plays. The start order should carry it (the server knows it).
- `controller::Tracks::Sources`, `players::Shared.{quality,discovery,streams}`, `StreamHost` on the server: delete.
- canon-934b mostly evaporates (clients stream `/audio`, which is already HiRes); what's left is the 96 kHz Cast resample and a per-player quality cap.
- canon-5100 drops in priority (serve no longer holds the LAN port) but stays: two clients on one machine still collide.
- canon-7196 becomes "serve + headless client on the always-on box".
- Docs: AGENTS.md's live-check recipe (test daemon + `control` + `sink Tunes`) needs a client in it; docs/firewall.md, docs/arch.md.

## Open questions

- Client command name: keep `tui` (names the UI, and a future native UI would be another binary), or `canon play` / `canon client`?
- One binary or two? One is one artifact to deploy; two (`canon` client, server separately) keeps sqlite out of the client, which canon-protocol was built for. Lean: one binary for now.
