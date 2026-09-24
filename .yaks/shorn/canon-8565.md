---
id: canon-8565
title: Probe connection capabilities and degrade on entitlement failures
type: task
priority: 2
created: '2026-09-24T21:28:11Z'
updated: '2026-09-24T21:53:45Z'
parent: canon-b3a5
depends_on:
- canon-c739
labels:
- arch
- tidal
verify: cargo test -p canon-tidal connector
---

Step 2. Probe at login/restore (Tidal: playbackinfo for a known track per quality -> the real Stream ceiling); a 4005/403 during playback downgrades verified capabilities and marks the connection Degraded, surfaced in the connections reply; routing falls through.

---
▸ 2026-09-24T21:53:37Z [Joel Webber]
Built: Health::Degraded{why} and ConnectionInfo.verified (best quality seen; a floor). TidalConnector keeps what playback and probes showed about the PKCE login (Observed): TidalSource handed out for Stream reports open() outcomes (Error::Auth from playbackinfo 401/403 = refused; an opened stream = streams at quality_of(info)); a refused login is excluded from session_for(Stream), so Sources answers NotEntitled with the hint "Tidal refused playback for the browser login (tidal.pkce): <why>. Sign in again with connect tidal.pkce". probe() resolves PROBE_TRACK 33348478 at startup (spawned in serve) and after a PKCE login; network errors / not found are inconclusive. Login and disconnect reset what was seen. canon control services shows degraded + verified.

---
▸ 2026-09-24T21:53:37Z [Joel Webber]
Live 2026-09-24, test daemon on :7399 over the real state dir: log "tidal streaming confirmed (Some(Lossless))"; services -> tidal.pkce signed in as 189763387, grants streaming up to hi_res, verified streaming up to lossless (Army of Me has no hi-res master, hence the floor wording). The refusal path is covered by a_refused_login_is_routed_around_and_says_why (fake HTTP answering playbackinfo 401 subStatus 4005): grants(Stream) false after a refused open, catalog still granted, health Degraded, verified stream None, hint says refused; re-login then probe refused again. A live refusal would need a login Tidal refuses (a device-code token), which there is no safe way to mint here.

---
▸ 2026-09-24T21:53:45Z [Joel Webber]
verify: `cargo test -p canon-tidal connector` -> PASS (exit 0)
