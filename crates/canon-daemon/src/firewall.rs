//! What a Linux firewall means for renderers (canon-6227).
//!
//! Speakers have to reach in: they fetch our stream over TCP, and DLNA devices answer our searches
//! with unicast UDP. Both go to `--lan-port`. A default-deny firewall drops them, and nothing
//! says so: Cast loads that never play, DLNA devices that never appear. ufw (on by default on
//! some distributions) keeps its state in world-readable files, so canon can tell whether a rule
//! admits the port, and say exactly what to run if none does. Other firewalls aren't read; the
//! "never fetched" error and docs/firewall.md cover them.

use std::path::Path;

/// What ufw does to inbound traffic on our port.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// ufw is off, or lets everything in.
    Open,
    /// ufw is on and a rule accepts the port for both protocols.
    Allowed,
    /// ufw is on and drops the port for these protocols.
    Blocked { tcp: bool, udp: bool },
    /// ufw is on and its rules couldn't be read.
    Unknown,
}

/// Log what ufw means for `port`, if ufw is in use. `network` is the LAN the rule should admit
/// (`192.168.0.0/24`), when known.
pub fn check(port: u16, network: Option<&str>) {
    let read = |path: &str| std::fs::read_to_string(Path::new(path)).ok();
    let Some(conf) = read("/etc/ufw/ufw.conf") else {
        return; // no ufw
    };
    let verdict = assess(
        &conf,
        read("/etc/default/ufw").as_deref(),
        read("/etc/ufw/user.rules").as_deref(),
        port,
    );
    let from = network.map_or_else(String::new, |network| format!("from {network} "));
    if port == 0 && verdict != Verdict::Open {
        tracing::warn!(
            "ufw is on, and --lan-port 0 picks a new port every time, which no rule can admit: \
             speakers won't be able to fetch canon's stream. Use a fixed port (docs/firewall.md)"
        );
        return;
    }
    match verdict {
        Verdict::Open => {}
        Verdict::Allowed => tracing::info!("ufw is on and admits port {port}"),
        Verdict::Blocked { tcp, udp } => {
            let (what, proto) = match (tcp, udp) {
                (true, true) => ("fetch canon's stream, and DLNA devices won't be found", ""),
                (true, false) => ("fetch canon's stream", " proto tcp"),
                _ => (
                    "answer DLNA searches, so DLNA devices won't be found",
                    " proto udp",
                ),
            };
            tracing::warn!(
                "ufw is on and drops port {port}: speakers can't {what}. Allow it once with: \
                 sudo ufw allow {from}to any port {port}{proto} (docs/firewall.md)"
            );
        }
        Verdict::Unknown => tracing::info!(
            "ufw is on and its rules can't be read here. If speakers can't reach canon, allow it \
             with: sudo ufw allow {from}to any port {port} (docs/firewall.md)"
        ),
    }
}

/// ufw's verdict on `port`, from its config (`ufw.conf`), its defaults (`/etc/default/ufw`), and
/// its rules (`user.rules`).
fn assess(conf: &str, defaults: Option<&str>, rules: Option<&str>, port: u16) -> Verdict {
    if setting(conf, "ENABLED").as_deref() != Some("yes") {
        return Verdict::Open;
    }
    // ufw's own default is to drop; only an explicit ACCEPT opens everything.
    if defaults
        .and_then(|d| setting(d, "DEFAULT_INPUT_POLICY"))
        .as_deref()
        == Some("ACCEPT")
    {
        return Verdict::Open;
    }
    let Some(rules) = rules else {
        return Verdict::Unknown;
    };
    let (tcp, udp) = (admits(rules, "tcp", port), admits(rules, "udp", port));
    if tcp && udp {
        Verdict::Allowed
    } else {
        Verdict::Blocked {
            tcp: !tcp,
            udp: !udp,
        }
    }
}

/// `KEY=value` (quotes optional) from a shell-style config file.
fn setting(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name.trim() == key).then(|| value.trim().trim_matches('"').to_string())
    })
}

/// Whether a rule in `user.rules` accepts inbound `proto` on `port`: an ACCEPT in `ufw-user-input`
/// that names no protocol or this one, and no port or one including this one. Where it accepts
/// from is not judged; a rule for the wrong network is the user's to see.
fn admits(rules: &str, proto: &str, port: u16) -> bool {
    rules.lines().any(|line| {
        let words: Vec<&str> = line.split_whitespace().collect();
        let after = |flag: &str| {
            words
                .iter()
                .position(|word| *word == flag)
                .and_then(|at| words.get(at + 1).copied())
        };
        let accepts = matches!(after("-j"), Some("ACCEPT" | "ufw-user-limit-accept"));
        if words.get(..2) != Some(&["-A", "ufw-user-input"]) || !accepts {
            return false;
        }
        let proto_ok = after("-p").is_none_or(|p| p == proto || p == "all");
        let port_ok = match after("--dport").or_else(|| after("--dports")) {
            None => true,
            Some(ports) => ports.split(',').any(|range| match range.split_once(':') {
                Some((low, high)) => {
                    low.parse().is_ok_and(|low: u16| low <= port)
                        && high.parse().is_ok_and(|high: u16| port <= high)
                }
                None => range.parse() == Ok(port),
            }),
        };
        proto_ok && port_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: &str = "# /etc/ufw/ufw.conf\nENABLED=yes\nLOGLEVEL=low\n";
    const DROP: &str =
        "IPV6=yes\nDEFAULT_INPUT_POLICY=\"DROP\"\nDEFAULT_OUTPUT_POLICY=\"ACCEPT\"\n";

    /// `user.rules` as ufw writes it, with `rules` in its RULES section.
    fn user_rules(rules: &str) -> String {
        format!(
            "*filter\n:ufw-user-input - [0:0]\n### RULES ###\n{rules}\n### END RULES ###\n\
             ### RATE LIMITING ###\n-A ufw-user-limit -j REJECT\n\
             -A ufw-user-limit-accept -j ACCEPT\n### END RATE LIMITING ###\nCOMMIT\n"
        )
    }

    #[test]
    fn off_or_accepting_ufw_is_open() {
        assert_eq!(
            assess("ENABLED=no\n", Some(DROP), None, 7346),
            Verdict::Open
        );
        let accept = "DEFAULT_INPUT_POLICY=\"ACCEPT\"\n";
        assert_eq!(assess(ON, Some(accept), None, 7346), Verdict::Open);
    }

    /// This machine's state when canon-6227 was found: on, dropping, no rules of the user's.
    #[test]
    fn no_rules_blocks_both_protocols() {
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules("")), 7346),
            Verdict::Blocked {
                tcp: true,
                udp: true
            }
        );
    }

    #[test]
    fn the_rules_ufw_writes_for_the_suggested_commands_are_recognised() {
        // sudo ufw allow from 192.168.0.0/24 to any port 7346
        let port = "### tuple ### allow any 7346 0.0.0.0/0 any 192.168.0.0/24 in\n\
             -A ufw-user-input -p tcp --dport 7346 -s 192.168.0.0/24 -j ACCEPT\n\
             -A ufw-user-input -p udp --dport 7346 -s 192.168.0.0/24 -j ACCEPT";
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules(port)), 7346),
            Verdict::Allowed
        );
        // sudo ufw allow from 192.168.0.0/24 to any app canon, exactly as ufw 0.36 wrote it on
        // the machine canon-6227 was found on: a single port, so plain --dport.
        let written = "### tuple ### allow tcp 7346 0.0.0.0/0 any 192.168.0.0/24 canon - in\n\
             -A ufw-user-input -p tcp --dport 7346 -s 192.168.0.0/24 -j ACCEPT -m comment --comment 'dapp_canon'\n\n\
             ### tuple ### allow udp 7346 0.0.0.0/0 any 192.168.0.0/24 canon - in\n\
             -A ufw-user-input -p udp --dport 7346 -s 192.168.0.0/24 -j ACCEPT -m comment --comment 'dapp_canon'";
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules(written)), 7346),
            Verdict::Allowed
        );
        // The multiport form ufw uses for a profile listing several ports.
        let app = "### tuple ### allow any any 0.0.0.0/0 any 192.168.0.0/24 canon - in\n\
             -A ufw-user-input -p tcp -m multiport --dports 7346 -s 192.168.0.0/24 -j ACCEPT -m comment --comment 'dapp_canon'\n\
             -A ufw-user-input -p udp -m multiport --dports 7346 -s 192.168.0.0/24 -j ACCEPT -m comment --comment 'dapp_canon'";
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules(app)), 7346),
            Verdict::Allowed
        );
        // sudo ufw allow from 192.168.0.0/24
        let network = "-A ufw-user-input -s 192.168.0.0/24 -j ACCEPT";
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules(network)), 7346),
            Verdict::Allowed
        );
    }

    #[test]
    fn a_rule_for_one_protocol_or_another_port_leaves_the_rest_blocked() {
        let tcp_only = "-A ufw-user-input -p tcp --dport 7346 -j ACCEPT";
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules(tcp_only)), 7346),
            Verdict::Blocked {
                tcp: false,
                udp: true
            }
        );
        let range = "-A ufw-user-input -p udp -m multiport --dports 22,7000:7400 -j ACCEPT\n\
             -A ufw-user-input -p tcp --dport 8096 -j ACCEPT";
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules(range)), 7346),
            Verdict::Blocked {
                tcp: true,
                udp: false
            }
        );
        let denied = "-A ufw-user-input -p tcp --dport 7346 -j DROP";
        assert_eq!(
            assess(ON, Some(DROP), Some(&user_rules(denied)), 7346),
            Verdict::Blocked {
                tcp: true,
                udp: true
            }
        );
    }

    #[test]
    fn unreadable_rules_are_unknown() {
        assert_eq!(assess(ON, None, None, 7346), Verdict::Unknown);
    }
}
