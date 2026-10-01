# canon

A headless music daemon: one self-contained Rust binary that owns your library, plays from the
services you subscribe to (Tidal, Spotify), and plays to local audio or to speakers on your
network (Chromecast, DLNA). Clients drive it over a WebSocket control plane: the terminal UI, a
line-oriented client for scripts, or anything else that speaks JSON.

The services are sources, not the source of truth. canon keeps its own library of tracks, albums,
artists and playlists, knows each one by its ISRCs and MusicBrainz ids, and binds it to every
service that has it, so a track imported from Spotify plays from Tidal when Tidal has it, and
plays count with the service you choose.

## Running it

```sh
cargo build --release
./target/release/canon serve              # the daemon, on 127.0.0.1:7345
./target/release/canon tui                # the terminal UI
```

Sign in from the TUI's Settings tab (or `canon control`: `connect tidal.pkce`), import your
favourites (`canon control`: `import`), and play. The daemon keeps its state
(credentials, the library, settings) in the platform's data directory, `~/Library/Application
Support/canon` on macOS; `--state-dir` puts it elsewhere.

On macOS, speakers fetch audio from canon, so the firewall must allow it incoming connections:
see [Signing dev builds](AGENTS.md#signing-dev-builds).

## Clients

- **`canon tui`**, the interactive terminal UI: [docs/tui.md](docs/tui.md).
- **`canon control`**, a line-oriented client, one command per line on stdin: made for scripts,
  agents and testing against real hardware (see [AGENTS.md](AGENTS.md)).
- **The control plane** itself: WebSocket + JSON at `ws://<host>:7345/ws`, specified in
  [`crates/canon-api/src/protocol.rs`](crates/canon-api/src/protocol.rs).

## Documentation

- [docs/arch.md](docs/arch.md): the shape of the system, and the decisions that shaped it.
- [docs/connections.md](docs/connections.md): how canon connects to each service, and what each
  kind of login can do.
- [docs/tui.md](docs/tui.md): the terminal UI.
- [docs/device-quirks.md](docs/device-quirks.md): what's known about particular speakers.
- [AGENTS.md](AGENTS.md): working agreements, the test harness, and pitfalls.

## Status

v0: playback from Tidal (up to hi-res FLAC) and Spotify (Premium, via librespot) to local audio,
Chromecast and DLNA, gapless; a library with matching across services and MusicBrainz
identification; import of favourites from both services; playlists, including browsing a
service's own playlists and mixes and copying or merging them into canon's (and export to
Spotify, which is the only service canon writes playlists to so far); recommendations and
autoplay; and the TUI. Local files, multi-room and a DSP chain are next; `yaks list` has the
rest.
