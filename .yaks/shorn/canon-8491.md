---
id: canon-8491
title: 'TUI sign-in: make the login URL easy to copy'
type: task
priority: 2
created: '2026-10-01T03:47:10Z'
updated: '2026-10-01T03:50:35Z'
labels:
- tui
verify: cargo test -p canon-tui sign_in
---

The sign-in popup draws the authorize URL inside a bordered, wrapped box, so selecting it with the mouse picks up border characters and padding unless the terminal does block selection. Draw the URL full-width with no side borders or indent (browsers drop the line breaks when it is pasted into the address bar), and copy it to the clipboard with OSC 52 when the sign-in starts (works over ssh, no X11/Wayland deps), with ctrl-y to copy again.

---
▸ 2026-10-01T03:50:35Z [Joel Webber]
Done. App gains a clipboard outbox (take_clipboard): set to the login URL when a sign-in starts (browser and device-code) and on ctrl-y in the popup (ctrl-y is no longer typed into the address box). The live runtime writes it as OSC 52 (ESC ]52;c;<base64> BEL) to stdout, so it works over ssh with no X11/Wayland deps; headless drops it. The popup is now full terminal width with top/bottom rules only, and the URL is cut into full rows with no indent, so selecting those rows yields just the URL (browsers drop the newlines on paste). Caveats: tmux needs set-clipboard on; the TUI can't know whether the terminal honoured it, so the popup says 'if your terminal allows it'.

---
▸ 2026-10-01T03:50:35Z [Joel Webber]
Live 2026-09-30 (Linux, signed-out test daemon :7399 with its own --state-dir). Headless TUI, key 6 / Enter on tidal.pkce: the URL drawn as three full 100-col rows starting at column 0 plus a short tail, no border beside it; typing abc then C-y left the input '> abc'. Real pty (script -c 'canon tui', keys 6, CR, ctrl-y): the output held two ESC]52;c; sequences, both decoding to https://login.tidal.com/authorize?response_type=code&redirect_uri=... (one at open, one for ctrl-y). NOT verified: pasting from a real terminal emulator's clipboard (needs Joel's terminal). Green bar: fmt, clippy 0 warnings, test all ok, build.

---
▸ 2026-10-01T03:50:35Z [Joel Webber]
verify: `cargo test -p canon-tui sign_in` -> PASS (exit 0)
