---
id: canon-6226
title: Settings as YAML in the platform config dir, populated on first start
type: task
priority: 2
created: '2026-10-01T03:37:34Z'
updated: '2026-10-01T03:40:40Z'
labels:
- settings
- daemon
verify: 'cargo test -p canon-daemon settings:: && cargo test -p canon-core settings::'
---

settings.json in the state dir has to be created by hand, and JSON is painful to edit. Move to settings.yaml in the platform config dir (directories' config_dir: ~/.config/canon on Linux, ~/Library/Application Support/canon on macOS), except that an explicit --state-dir keeps it inside that dir (isolated test/signed-out daemons). On first start write a commented file with every setting, migrating an existing settings.json (left in place for older builds). Every write re-renders the commented document, so comments survive set_settings. Point the Spotify no-client-id error at the real path.

---
▸ 2026-10-01T03:40:31Z [Joel Webber]
Done. settings.yaml (serde_norway, the maintained serde_yaml fork) at directories' config_dir (~/.config/canon on Linux, honours XDG_CONFIG_HOME; ~/Library/Application Support/canon on macOS); an explicit --state-dir/CANON_STATE_DIR keeps it in that dir. Missing file -> written at startup with every key and a comment per section, copying <state_dir>/settings.json if present (JSON parses as YAML; the old file is left for older builds). Every set re-renders the commented document. SettingsStore::location() lets the Spotify no-client-id error name the real path. A file that parses is never rewritten on load; one that doesn't still stops the daemon. Hot reload of hand edits is NOT done: the header says edit while canon is stopped.

---
▸ 2026-10-01T03:40:31Z [Joel Webber]
Live 2026-09-30 (Linux, test daemon :7399, real state dir, XDG_CONFIG_HOME=/tmp/canon-xdg-6226 so ~/.config was untouched): log 'settings: copying /home/joel/.local/share/canon/settings.json to /tmp/canon-xdg-6226/canon/settings.yaml'; file had all five sections with autoplay: true carried over and 'client_id:' blank. connect spotify.web -> 'error: auth: no Spotify client id: ... then set spotify.client_id in /tmp/canon-xdg-6226/canon/settings.yaml (or with set_settings) ...'. autoplay off -> file shows autoplay: false, all 23 comment lines kept; legacy settings.json unchanged (autoplay true). Second daemon with --state-dir /tmp/canon-state-6226 -> 'settings: /tmp/canon-state-6226/settings.yaml'. Green bar: fmt, clippy (0 warnings), test (all ok), build.

---
▸ 2026-10-01T03:40:40Z [Joel Webber]
verify: `cargo test -p canon-daemon settings:: && cargo test -p canon-core settings::` -> PASS (exit 0)
