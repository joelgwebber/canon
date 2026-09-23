//! Serve the LAN stream server on a real LAN interface and fetch it (yak canon-21f7).
//!
//! Loopback proves the HTTP shape; this proves the *interface* bind a renderer actually connects
//! to. Ignored by default because it depends on the host having a usable LAN NIC.
//!
//! Run with: `cargo test -p canon-sink --test stream_server_lan -- --ignored --nocapture`
//!
//! ## macOS: expect a reset unless the binary is allowed through the firewall
//!
//! The macOS Application Firewall auto-allows only *signed* software. A cargo-built binary is
//! ad-hoc/linker-signed, so its inbound connections are reset (loopback is never filtered, which
//! is why the sibling loopback test still passes). Allow it explicitly:
//!
//! ```text
//! sudo /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp <path to the test binary>
//! ```
//!
//! Per-test binaries live under `target/debug/deps/<name>-<hash>` and get a new hash — and so a
//! new identity — on every rebuild, so that entry must be re-added; `target/debug/canon` is a
//! stable path and stays allowed. A shipped build should be properly signed instead.

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use canon_sink::StreamRoutes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
#[ignore = "requires a LAN interface (and macOS Local Network permission)"]
async fn serving_on_the_lan_interface_is_reachable() {
    let interfaces = canon_sink::host_interfaces();
    let usable = canon_sink::usable_interfaces(&interfaces);
    let lan = usable
        .iter()
        .find(|i| i.ip.is_ipv4())
        .expect("a usable IPv4 LAN interface");
    println!("binding on {} ({})", lan.ip, lan.name);

    let routes = StreamRoutes::default();
    let (path, broadcaster) = routes.open();
    broadcaster.set_header(Bytes::from_static(b"fLaC-header"));
    let (bound, _server) = canon_sink::spawn(SocketAddr::new(lan.ip, 0), routes.clone())
        .await
        .expect("spawn stream server on the LAN interface");
    println!("bound: {bound}");
    broadcaster.push(Bytes::from_static(b"FRAME-1"));

    let mut socket = TcpStream::connect(bound)
        .await
        .expect("connect to our own LAN-bound server");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {bound}\r\n\r\n");
    socket.write_all(request.as_bytes()).await.expect("write");

    let mut buffer = vec![0u8; 4096];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
        .await
        .expect("responded before timeout")
        .unwrap_or_else(|e| {
            // The overwhelmingly likely cause on macOS is the Application Firewall blocking this
            // unsigned binary's inbound connections; say so rather than leaving a bare errno.
            panic!(
                "read failed: {e}\n\
                 On macOS this is usually the Application Firewall blocking inbound connections \
                 to this unsigned test binary. Allow it with:\n  sudo \
                 /usr/libexec/ApplicationFirewall/socketfilterfw --unblockapp \
                 $(pwd)/target/debug/deps/<this-test-binary>"
            )
        });
    let response = String::from_utf8_lossy(&buffer[..read]).to_string();
    println!("response head: {:?}", &response[..response.len().min(200)]);
    assert!(response.starts_with("HTTP/1.1 200"), "got: {response:?}");
    assert!(response.contains("fLaC-header"), "got: {response:?}");
}
