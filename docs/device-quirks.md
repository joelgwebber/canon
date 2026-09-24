# Device quirks

What real renderers do that no spec or unit test told us. Each entry was paid for on metal; the
yak ids carry the evidence. `AGENTS.md` explains why unit tests alone are not grounds for shearing
anything that touches a device — this file is the memory of what those devices taught us.

Keep entries to: **what the device does**, **how it shows up**, **what canon does about it**, and
the yak with the evidence. Name the *layer* that owns the behaviour (a speaker often runs several
independent stacks: the LS50 Wireless II's Cast and DLNA sides share nothing but the amplifier).

## KEF LS50 Wireless II ("Tunes")

Reachable over Cast (HTTP user agent `Cast Lite`) and DLNA (Rygel for control, GStreamer
`souphttpsrc` for the fetch). The two ids are unrelated; canon groups them into one output.

### Cast: drops our live stream after minutes — use DLNA

- **Behaviour.** The Cast player reads a *finite* resource in range bursts: it fills ~9.4 MB
  (~90 s of 16/44.1 FLAC), closes the connection itself, and comes back ~80–100 s later with
  `Range: bytes=<where it stopped>-`. Our live stream is an unbounded chunked body with no
  `Content-Length` and no `Accept-Ranges`, so it cannot be resumed that way — the likeliest reason
  the player gives up on it. Somewhere between 2 and 14 minutes in (any track, flow or not),
  playback stalls; ~25–30 s later the player closes our connection and discards the media without
  ever retrying. What triggers the stall is not visible from our side.
- **How it shows up.** Audio goes silent while the receiver keeps reporting `PLAYING` with
  `current_time` crawling at ~0.8× (with a deep producer lead it instead plays on from its buffer and
  quits abruptly). Then the media status comes back empty, which canon reads as the track ending: the
  queue skips to the next track, which LOADs a fresh stream and plays normally. The only log trace is
  `stream server: consumer gone … ended_by_us=false` followed by `cast load accepted`.
- **Ruled out** (canon-c200): ring lag, feed pacing, Tidal segment fetches, FLAC validity (a tap of
  the same stream decodes clean), Wi-Fi (TCP send queue ~0, ping 4–43 ms through a crawl), producer
  lead depth (2 s and 30 s both fail), Cast status-poll rate (500 ms and 5 s both fail), a receiver
  app relaunch, and Spotify desktop holding its own Cast connection.
- **Controls.** The same stream and code play cleanly to Tunes over **DLNA** (flow, 24 min, 5
  gapless joins) and to a Google Nest Hub over **Cast** (flow, 29 min, 6 gapless joins). A finite
  FLAC served with `Content-Length` and ranges plays cleanly to Tunes over Cast (canon-c200 E3:
  40 min, 28 range fetches), where our live stream never lasted past ~13.5 min. Tidal's and
  tideway's own casts to the KEF have never shown it.
- **What to do.** Prefer DLNA for this speaker. Casting to it needs a finite, range-capable resource
  per load, which our live FLAC (unknown encoded length) and flow mode (one endless stream) are not.

### Cast: refuses to launch while leaving DLNA

A Cast `LAUNCH` while the speaker is playing (or just released) DLNA is answered with `CANCELLED`,
and the attempt knocks the DLNA playback over. canon releases the speaker before switching
protocols on it and retries the connect up to 3 times, 1 s apart (`connect_settled`). canon-7c6f.

### DLNA: GStreamer takes STREAMINFO frame sizes at their word

GStreamer's `flacparse` read flacenc's placeholder minimum frame size (16 MiB) literally and waited
for 16 MiB before parsing anything — `TRANSITIONING` forever. The FLAC tap writes 0/0 ("unknown").
canon-685a.

### DLNA: a probe connection precedes every fetch

Each load opens the stream twice: a Rygel `GET` that closes immediately (0 bytes), then the
GStreamer fetch. Anything keyed on "the first consumer" or on connection counts must allow for it.
canon-1922, canon-c200.

### DLNA: `RelTime` is whole seconds, stream-relative

Position reports truncate to the second (reads as a steady ~0.5 s lag) and are relative to the
stream URI, like Cast's. Corrections slew backward more gently than forward. canon-bc84, canon-685a.

### Any protocol: an invisible Spotify Connect takeover

Spotify Connect can take the speaker while every Cast status poll still reports our session
`PLAYING`. Only our stream losing its consumer reveals it; that is what the watchdog keys on.
canon-2dbf.

## Cast receivers in general

- **The end of a track is an empty status, not `FINISHED`.** `IDLE`/`FINISHED` is broadcast once,
  unsolicited, between polls, and is lost; afterwards every status has no entries. canon-e920,
  canon-a70f.
- **A stop reads as `IDLE`/`CANCELLED`,** which must not be taken for the track ending. canon-44f4.
- **Positions are relative to the stream the renderer was handed,** so after a seek they start at
  the seek point. canon-bc84.

## Google Nest Hub ("Library display", "Kitchen")

- Plays our live chunked FLAC stream in flow mode over Cast without the KEF's drop (29 min,
  6 gapless joins, canon-c200 E2).
