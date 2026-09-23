---
id: canon-2dbf
title: Detect non-Cast takeover of a renderer (Spotify Connect / AirPlay)
type: bug
priority: 2
created: '2026-09-23T02:49:47Z'
updated: '2026-09-23T03:06:21Z'
parent: canon-7718
labels:
- sink,network
verify: cargo test --workspace
---

OBSERVED ON METAL: while canon was casting to the KEF LS50 Wireless II ('Tunes'), Spotify took the speaker over via SPOTIFY CONNECT (the speaker's native protocol), not Chromecast. Our audio stopped coming out, but canon kept reporting state=playing sink=Tunes with position advancing — the exact tideway 'asserting stale control' desync this family exists to prevent.
WHY classify() missed it (it is NOT a classify bug): the Cast MEDIA session was never disturbed. 267 consecutive status polls still returned OUR media_session_id=1, our stream URL, player_state=Playing, and the RECEIVER-level status still listed only our Default Media Receiver app (app_id=CC1AD845) with no Spotify app. The Cast namespace genuinely cannot see a Connect/AirPlay takeover. classify()'s foreign-session and IdleReason::Interrupted paths remain correct for a CAST takeover (still unit-tested, still unobserved live).
Evidence tool added: tests/cast_receiver_probe.rs (ignored; CANON_CAST_ADDR=host:8009) dumps receiver-level app list/volume/active-input.
CANDIDATE SIGNALS to investigate, cheapest first: (1) our LAN stream server sees the receiver's HTTP connection CLOSE when the speaker switches source — we already log consumer-connect, so track consumer COUNT and treat 'zero consumers while we believe we are casting' as the authoritative liveness signal (protocol-agnostic, works for DLNA too, and is a pure canon-21f7-level fact); (2) receiver volume/active-input changes; (3) device-specific: KEF/Spotify Connect state via its own API. Prefer (1) — it keys on whether our bytes are actually being consumed, which IS the ground truth for 'is our audio reaching the speaker'.

---
▸ 2026-09-23T03:06:10Z [Joel Webber]
verify: `cargo test --workspace` -> PASS (exit 0)

---
▸ 2026-09-23T03:06:20Z [Joel Webber]
FIXED + CONFIRMED ON METAL (user grabbed Tunes with Spotify Connect while canon was casting to it).
FIX: keyed detection on whether our bytes are actually being CONSUMED, not on what the control protocol claims. StreamBroadcaster::consumers() (broadcast receiver_count) is protocol-agnostic ground truth; the controller runs a stream watchdog per cast session that fails back to local after CONSUMER_GRACE (10s) with zero consumers, resetting the timer if a consumer returns (a brief drop is also how a renderer RECONNECTS, which header replay is built to serve). Chose this over receiver-volume/active-input or device-specific APIs because it works for any renderer and any competing protocol — it will cover DLNA (canon-685a) unchanged.
EVIDENCE (live, KEF 'Tunes'):
  t+  0s state=playing sink=CAST  pos=0
  t+ 71s state=loading sink=local pos=0
  t+ 71s state=playing sink=local pos=71887   <- resumed locally at the SAME position
  t+234s state=ended   sink=local pos=234332  <- played the rest of the track through to the end
  WARN canon::controller: renderer stopped consuming our stream (10s); assuming the session was taken over and failing back to local sink=LS50-Wireless-II-...
So the tideway desync is gone: canon no longer claims to be playing to a speaker it has lost — it notices on its own and keeps the music going locally.
Also added tests/cast_receiver_probe.rs (ignored diagnostic) which is what PROVED the Cast layer is blind here: the receiver still listed only our own Default Media Receiver app (CC1AD845) with no Spotify app, and 267 consecutive media polls still reported our media_session_id=1 Playing. classify()'s Cast-native takeover paths remain correct and unit-tested (a CAST takeover is still unobserved live).
Unit test: stream_server::tests::consumer_count_tracks_live_subscribers.
