//! A startup check for the one macOS setting that silently breaks network playback: the
//! Application Firewall refusing incoming connections to this build (yak canon-041e).
//!
//! Renderers fetch our stream from us, and DLNA discovery listens for replies, so both need
//! incoming connections. The firewall grants them per app, recorded against the binary's code
//! signature. An ad-hoc signed build (the toolchain's default) is identified by its code hash,
//! which every rebuild changes, so the grant stops matching without a word: Cast speakers accept
//! a load and never fetch it, and DLNA devices vanish. The firewall's own `--getappblocked`
//! answers by path and says "permitted" all the while, so the signature is what's checked here.

use std::path::Path;
use std::process::Command;

const SOCKETFILTERFW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";

/// Log what the firewall and this build's signature mean for renderers. Blocking; run it off the
/// async threads.
pub fn check_firewall() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if !firewall_enabled() {
        return;
    }
    match signing_authority(&exe) {
        Signature::AdHoc => tracing::warn!(
            "the macOS firewall is on and this canon ({}) is ad-hoc signed: its \"allow incoming \
             connections\" grant is lost on every rebuild, and then speakers can't fetch our \
             stream and DLNA discovery hears nothing. Sign builds with a stable identity \
             (AGENTS.md, \"Signing dev builds\")",
            exe.display()
        ),
        Signature::Identity(authority) => tracing::info!(
            "the macOS firewall is on; this canon is signed as \"{authority}\", so one grant lasts \
             across rebuilds. If speakers can't fetch our stream, allow it once: sudo {SOCKETFILTERFW} \
             --add {exe} && sudo {SOCKETFILTERFW} --unblockapp {exe}",
            exe = exe.display()
        ),
        Signature::Unknown => {}
    }
}

fn firewall_enabled() -> bool {
    Command::new(SOCKETFILTERFW)
        .arg("--getglobalstate")
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("enabled"))
}

enum Signature {
    AdHoc,
    Identity(String),
    Unknown,
}

fn signing_authority(exe: &Path) -> Signature {
    let Ok(out) = Command::new("codesign").arg("-dvv").arg(exe).output() else {
        return Signature::Unknown;
    };
    // codesign reports on stderr.
    parse_signature(&String::from_utf8_lossy(&out.stderr))
}

fn parse_signature(report: &str) -> Signature {
    if report.lines().any(|line| line.trim() == "Signature=adhoc") {
        return Signature::AdHoc;
    }
    report
        .lines()
        .find_map(|line| line.strip_prefix("Authority="))
        .map_or(Signature::Unknown, |authority| {
            Signature::Identity(authority.trim().to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_is_read_from_codesigns_report() {
        let adhoc = "Executable=/x/canon\nIdentifier=canon-1f2e\nFormat=Mach-O thin (arm64)\n\
                     CodeDirectory v=20400 size=1 flags=0x20002(adhoc,linker-signed)\n\
                     Signature=adhoc\nTeamIdentifier=not set\n";
        assert!(matches!(parse_signature(adhoc), Signature::AdHoc));
        let signed = "Executable=/x/canon\nIdentifier=canon\nAuthority=canon dev\n\
                      Signature size=1644\nTeamIdentifier=not set\n";
        assert!(matches!(parse_signature(signed), Signature::Identity(a) if a == "canon dev"));
        assert!(matches!(parse_signature("garbage"), Signature::Unknown));
    }
}
