---
id: canon-8b0b
title: 'TUI phase 3: outputs, services and settings'
type: task
priority: 3
created: '2026-09-27T17:42:31Z'
updated: '2026-09-27T19:13:05Z'
parent: canon-3db9
labels:
- tui
verify: cargo test -p canon-tui
---

Output picker (select, per-output mode), services with their connections and sign-in, streaming order and autoplay.

![tui-settings-session](artifacts/canon-8b0b/tui-settings-session.txt)

![tui-outputs-session](artifacts/canon-8b0b/tui-outputs-session.txt)

---
▸ 2026-09-27T19:13:05Z [Joel Webber]
Live (test daemon :7399, real state, nothing playing, 2026-09-27; sessions attached): Outputs listed Local, Basement, Kitchen, Library display, Tunes (cast · dlna, with 'over cast'/'over dlna' rows) and the Living Room TV, each with its mode; Enter on Tunes => 'playing on Tunes' and ● on Tunes/over cast; 'over dlna' => ● moved to dlna; Local => back. Settings: tidal.pkce/spotify.web/spotify.librespot signed in, tidal.device not; Enter on tidal.device showed the device-code overlay (link.tidal.com/JKMFZ, code JKMFZ), Esc cancelled; autoplay off then on and streaming order swapped and swapped back, each confirmed by the daemon's read-back; settings.json ends as it began (autoplay true, default order). Sign-out not exercised live (it would drop Joel's real credentials); unit-tested. 20 canon-tui tests (4 new), frames reviewed.

---
▸ 2026-09-27T19:13:05Z [Joel Webber]
verify: `cargo test -p canon-tui` -> PASS (exit 0)
