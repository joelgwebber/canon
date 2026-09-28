---
id: canon-4fcb
title: Freeze on transient tidal request error
type: bug
priority: 3
created: '2026-09-28T12:47:29Z'
updated: '2026-09-28T14:06:27Z'
verify: cargo test -p canon-tidal transient_segment_failures_are_retried && cargo test -p canon-core mid_way
---

Looks like a transient Tidal error caused the queue to stop altogether:

```
2026-09-28T12:45:32.366454Z  INFO canon: player state seq=17 state=Playing position_ms=279065
2026-09-28T12:45:32.373552Z  WARN canon_tidal::segment: segment reader: source failed: transient: tidal http GET https://sp-ad-cf.audio.tidal.com/mediatracks/GisIAxInNTAxZGMzYmYwNzY0ZWZjMWQ5Y2UyYTA3NDZmNjFjZmJfNjEubXA0IiAdAACAQCACKhAN2EkadO7zIW-D-knjT3MQMgUNAACgQQ/71.mp4?Policy=eyJTdGF0ZW1lbnQiOlt7IlJlc291cmNlIjoiaHR0cHM6Ly9zcC1hZC1jZi5hdWRpby50aWRhbC5jb20vbWVkaWF0cmFja3MvR2lzSUF4SW5OVEF4WkdNelltWXdOelkwWldaak1XUTVZMlV5WVRBM05EWm1OakZqWm1KZk5qRXViWEEwSWlBZEFBQ0FRQ0FDS2hBTjJFa2FkTzd6SVctRC1rbmpUM01RTWdVTkFBQ2dRUS8qIiwiQ29uZGl0aW9uIjp7IkRhdGVMZXNzVGhhbiI6eyJBV1M6RXBvY2hUaW1lIjoxNzkwNjAyODIzfX19XX0_&Signature=EHg7cKF1XZJDVXfzh8Oyiw6oUik0YZGtmAZVSXYRBKtgyS7IVeORNpjj20oUWNPTf65titGmwyaGfZWbibDPyyNHDUSLc0ohoXBTw6mSYQbmdB8XOq-kb60fWPunPFgU8mVKtN4xILzVgECv9SES3Atkkvh2ZRiK-J6YoSHWcrMjUWomXhg~bBydAvuCUfy6u9Da3w5TxzWtnm8U9KnbUz-ROv-LL~OBPdZ-AeTyCVqJFdzudGI~2H6V~zRhh0KCWArHjYFLQuykqrvRj6jm~NbO4JSl7R1m8a--x1O2fEb9YFaV3GfcwijfS2tbbMSX5oDIklzkd95gG18CQYqBiA__&Key-Pair-Id=K14LZCZ9QUI4JL: error sending request for url (https://sp-ad-cf.audio.tidal.com/mediatracks/GisIAxInNTAxZGMzYmYwNzY0ZWZjMWQ5Y2UyYTA3NDZmNjFjZmJfNjEubXA0IiAdAACAQCACKhAN2EkadO7zIW-D-knjT3MQMgUNAACgQQ/71.mp4?Policy=eyJTdGF0ZW1lbnQiOlt7IlJlc291cmNlIjoiaHR0cHM6Ly9zcC1hZC1jZi5hdWRpby50aWRhbC5jb20vbWVkaWF0cmFja3MvR2lzSUF4SW5OVEF4WkdNelltWXdOelkwWldaak1XUTVZMlV5WVRBM05EWm1OakZqWm1KZk5qRXViWEEwSWlBZEFBQ0FRQ0FDS2hBTjJFa2FkTzd6SVctRC1rbmpUM01RTWdVTkFBQ2dRUS8qIiwiQ29uZGl0aW9uIjp7IkRhdGVMZXNzVGhhbiI6eyJBV1M6RXBvY2hUaW1lIjoxNzkwNjAyODIzfX19XX0_&Signature=EHg7cKF1XZJDVXfzh8Oyiw6oUik0YZGtmAZVSXYRBKtgyS7IVeORNpjj20oUWNPTf65titGmwyaGfZWbibDPyyNHDUSLc0ohoXBTw6mSYQbmdB8XOq-kb60fWPunPFgU8mVKtN4xILzVgECv9SES3Atkkvh2ZRiK-J6YoSHWcrMjUWomXhg~bBydAvuCUfy6u9Da3w5TxzWtnm8U9KnbUz-ROv-LL~OBPdZ-AeTyCVqJFdzudGI~2H6V~zRhh0KCWArHjYFLQuykqrvRj6jm~NbO4JSl7R1m8a--x1O2fEb9YFaV3GfcwijfS2tbbMSX5oDIklzkd95gG18CQYqBiA__&Key-Pair-Id=K14LZCZ9QUI4JL): client error (SendRequest) at=25106092
2026-09-28T12:45:32.403229Z  INFO canon: player state seq=18 state=Error position_ms=0
2026-09-28T12:45:59.030531Z  INFO canon: player state seq=19 state=Loading position_ms=0
2026-09-28T12:45:59.094444Z  INFO canon::controller: playing "Send And Receive" from tidal:33175638 (flac 16/44.1 kHz)
```

We should at least keep going, but even better if there's a way to recover from such an error, perhaps with a fallback-retry loop.

---
▸ 2026-09-28T14:06:22Z [Joel Webber]
Fix (2026-09-28): (1) canon-tidal: segment fetches retry a transient failure (Error::Transient from the HTTP client, i.e. a request with no reply like Joel's 'client error (SendRequest)', and now HTTP 5xx/429) after 0.25/0.5/1/2/4/8s (~16s, inside the ~10s network lead + 3 segments of read-ahead), warning 'segment fetch failed (...); retrying in ...'. 404 etc. are not retried. (2) player: a Failed while Playing/Paused restarts the same entry once at the listener's position ('restarting "<title>" where it was: ...'); failing again it is skipped under canon-118a's MAX_SKIPS. Tests: transient_segment_failures_are_retried, a_track_that_fails_mid_way_is_restarted_once_then_skipped.

---
▸ 2026-09-28T14:06:22Z [Joel Webber]
Live (muted, vol 0, test daemon :7399, local output): tracks opened and decoded through the retrying fetch path, but the Mac's default output (Joel in a meeting) refused: 'device has no f32 output config for 2 channels', so nothing played and a transient failure could not be forced (no way to break the CDN without sudo/pf). That run also shows canon-118a's skip catching output failures: bounded by MAX_SKIPS, but each costs the engine's ~10s device reopen, so a dead local output takes ~50s to stop. Worth a follow-up if it bites.

---
▸ 2026-09-28T14:06:27Z [Joel Webber]
verify: `cargo test -p canon-tidal transient_segment_failures_are_retried && cargo test -p canon-core mid_way` -> PASS (exit 0)
