//! Diagnostic: ask a Cast receiver what it thinks is running (yak canon-dde4).
//!
//! Answers "who owns the speaker right now?" straight from the device: the receiver-level status
//! lists the running applications plus volume and active-input flags, which is the layer that can
//! reveal a takeover by a *non-Cast* protocol (Spotify Connect, AirPlay, an analog input) that the
//! media namespace never sees.
//!
//! Run with:
//! `CANON_CAST_ADDR=192.168.0.205:8009 cargo test -p canon-sink --test cast_receiver_probe -- --ignored --nocapture`

use std::net::SocketAddr;

use rust_cast::CastDevice;

#[test]
#[ignore = "requires a real Cast device; set CANON_CAST_ADDR"]
fn dump_receiver_status() {
    let addr: SocketAddr = std::env::var("CANON_CAST_ADDR")
        .expect("set CANON_CAST_ADDR, e.g. 192.168.0.205:8009")
        .parse()
        .expect("CANON_CAST_ADDR must be host:port");

    let device = CastDevice::connect_without_host_verification(addr.ip().to_string(), addr.port())
        .expect("connect to the cast device");
    device
        .connection
        .connect("receiver-0")
        .expect("connect channel");

    let status = device.receiver.get_status().expect("receiver status");
    println!("--- receiver status ---");
    println!("is_active_input: {}", status.is_active_input);
    println!("is_stand_by:     {}", status.is_stand_by);
    println!(
        "volume:          level={:?} muted={:?}",
        status.volume.level, status.volume.muted
    );
    println!("applications ({}):", status.applications.len());
    for app in &status.applications {
        println!(
            "  app_id={} display_name={:?} status_text={:?} session_id={} transport_id={}",
            app.app_id, app.display_name, app.status_text, app.session_id, app.transport_id
        );
        println!("    namespaces: {:?}", app.namespaces);
    }
}
