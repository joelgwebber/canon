---
id: canon-bc4f
title: Sign dev builds with a stable local identity, so firewall and Local Network grants survive rebuilds
type: task
priority: 1
created: '2026-09-27T18:27:22Z'
updated: '2026-09-28T00:27:26Z'
parent: canon-f495
labels:
- macos
verify: codesign -dr - target/release/canon 2>&1 | grep -q "certificate leaf"
---

Ad-hoc signed builds are identified by cdhash, which changes every build, so the macOS Application Firewall's allow rule silently stops matching (canon-041e). Joel chose (2026-09-27): a self-signed 'canon dev' code-signing certificate in the login keychain, plus a linker wrapper in .cargo/config.toml that codesigns every linked macOS binary with it, giving a designated requirement (identifier + certificate) that is stable across rebuilds. One re-allow afterwards.

---
▸ 2026-09-27T18:36:49Z [Joel Webber]
Built 2026-09-27: 'canon dev' self-signed code-signing identity imported into the login keychain (-T /usr/bin/codesign; the key files were deleted after import), scripts/link-and-sign named as the linker for aarch64/x86_64-apple-darwin in .cargo/config.toml. Every build now: codesign -dr - target/debug/canon => 'identifier canon and certificate leaf = H"f6ef16a251512b8760f175f5ae205e64c77cfa21"', unchanged across rebuilds. One signing attempt failed with 'internal error in Code Signing subsystem' (probably a pending keychain access prompt), then worked. Owed: Joel grants the signed binary once (sudo socketfilterfw --add/--unblockapp), then verify a *rebuilt* binary still streams to Tunes and sees DLNA without a new grant.

---
▸ 2026-09-27T18:43:44Z [Joel Webber]
Verified 2026-09-27 after Joel's one grant: rebuilt (touch main.rs) => DR still 'certificate leaf = H"f6ef…"'; with no new grant, canon devices lists 'Dlna Tunes 192.168.0.205:16500' and '[TV] Living Room TV' again, and a test daemon on :7399 to Tunes@cast logged 'stream server: consumer connected' then state=Playing (vol 0). Joel confirmed his own daemon plays to Tunes.

---
▸ 2026-09-27T18:43:44Z [Joel Webber]
verify: `codesign -dr - target/debug/canon 2>&1 | grep -q "certificate leaf"` -> PASS (exit 0)

---
▸ 2026-09-27T19:32:58Z [Joel Webber]
Regrown 2026-09-27: release builds were never signed. Tunes silent over cast and dlna from Joel's daemon (target/release/canon, built 15:18): codesign -dr - says cdhash H"d7719e…", flags adhoc,linker-signed. Cause: cargo's release profile defaults to strip = "debuginfo", and on macOS rustc runs strip after the linker returns, which re-signs ad hoc over ours. Reproduced with rustc on an empty main: -C strip=none keeps "certificate leaf", -C strip=debuginfo gives cdhash. The firewall grant for target/release/canon is listed by path but matches a stale identity.

---
▸ 2026-09-27T19:35:38Z [Joel Webber]
verify: `codesign -dr - target/release/canon 2>&1 | grep -q "certificate leaf"` -> PASS (exit 0)

---
▸ 2026-09-27T19:35:38Z [Joel Webber]
Fixed with [profile.release] strip = "none". cargo build --release -p canon-daemon => codesign -dr - target/release/canon: designated => identifier canon and certificate leaf = H"f6ef16a251512b8760f175f5ae205e64c77cfa21". Green bar clean. Also found: target/debug/canon has no entry in socketfilterfw --listapps any more (only target/release/canon, stale), and a signed debug test daemon on :7399 got "cast load: Failed to load media. It never fetched a stream from http://192.168.0.67:52462" and never discovered Tunes over dlna. Caution: rebuilding release killed the running daemon (its binary was rewritten in place), so stop the daemon before a release build.

---
▸ 2026-09-27T19:35:38Z [Joel Webber]
Please grant the now-signed release binary once: sudo /usr/libexec/ApplicationFirewall/socketfilterfw --add /Users/joel/src/canon/target/release/canon && sudo /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp /Users/joel/src/canon/target/release/canon (and the same for target/debug/canon if you want test daemons to reach speakers). Then restart your daemon; I owe an on-metal check that Tunes plays over cast and dlna, and again after a rebuild with no new grant.

---
▸ 2026-09-27T21:22:12Z [Joel Webber]
Done!
