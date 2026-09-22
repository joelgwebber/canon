//! Fingerprint verification for the `wreq` impersonation backend (yak canon-a880).
//!
//! This is `#[ignore]`d because it needs outbound network to a TLS-fingerprint reflector,
//! and this workspace's terminal is sandboxed with per-host grants. Run it explicitly:
//!
//! ```text
//! cargo test -p canon-tidal --test fingerprint -- --ignored --nocapture
//! ```
//!
//! It issues an HTTPS GET through the Chrome-on-Android emulation preset to a reflector
//! and prints the JA3/JA4 + HTTP/2 fingerprint it observed. Eyeball the output: a real
//! Chrome/Android profile has a GREASEd JA3 with many extensions and a JA4 beginning
//! `t13d…`, distinct from a default rustls hello.
//!
//! If the reflector host is not granted, the request errors with a connection failure —
//! that is a *network-not-granted* result, not a fingerprint failure. The build+API
//! evidence (this file compiling against `WreqHttp::chrome_android`) stands regardless.

use canon_tidal::{TidalHttp, WreqHttp};

// Candidate reflectors (any one is enough). tls.peet.ws returns ja3/ja4 + akamai h2 fp.
const REFLECTOR: &str = "https://tls.peet.ws/api/all";

#[tokio::test]
#[ignore = "needs outbound network to a TLS-fingerprint reflector; run with --ignored"]
async fn chrome_android_presents_browser_fingerprint() {
    let http = WreqHttp::chrome_android().expect("build chrome-android wreq client");
    println!("emulation profile: {}", http.profile());

    let resp = http
        .get(REFLECTOR, &[("accept", "application/json")])
        .await
        .expect("GET reflector (if this is a connection error, the host was not granted)");

    println!("HTTP {}", resp.status);
    let text = resp.text().expect("utf8 body");
    // Print the reflector's view of our TLS/HTTP2 fingerprint for manual inspection.
    println!("---- reflector response ----\n{text}\n----------------------------");

    assert!(resp.is_success(), "reflector returned non-2xx");
    // Cheap sanity: the reflector echoes a ja3/ja4 field when it saw a real TLS hello.
    let lowered = text.to_lowercase();
    assert!(
        lowered.contains("ja3") || lowered.contains("ja4"),
        "reflector response did not contain a ja3/ja4 field"
    );
}
