//! A real socket fetch against the LAN stream server (yak canon-21f7).
//!
//! The in-crate tests drive the router through `tower::oneshot`, which never touches TCP. A real
//! renderer does, so this test binds the server on loopback and performs an actual HTTP/1.1 GET
//! over a socket — the level at which "the server accepted and then reset the connection" shows
//! up. Caught exactly that class of bug on the first live cast attempt.

use std::time::Duration;

use bytes::Bytes;
use canon_sink::{STREAM_PATH, StreamBroadcaster};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[tokio::test]
async fn a_real_socket_get_receives_the_header() {
    let broadcaster = StreamBroadcaster::default();
    broadcaster.set_header(Bytes::from_static(b"fLaC-header"));

    let addr: std::net::SocketAddr = "127.0.0.1:0".parse().expect("addr");
    let (bound, _server) = canon_sink::spawn(addr, broadcaster.clone())
        .await
        .expect("spawn stream server");

    // Push a frame so the live edge has content beyond the header.
    broadcaster.push(Bytes::from_static(b"FRAME-1"));

    let mut socket = TcpStream::connect(bound).await.expect("connect");
    let request = format!("GET {STREAM_PATH} HTTP/1.1\r\nHost: {bound}\r\n\r\n");
    socket
        .write_all(request.as_bytes())
        .await
        .expect("write request");

    // Read enough to cover the status line, headers, and the first chunk(s) of body.
    let mut buffer = vec![0u8; 4096];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer))
        .await
        .expect("server responded before the timeout")
        .expect("read response");
    let response = String::from_utf8_lossy(&buffer[..read]).to_string();

    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected a 200 over HTTP/1.1, got: {response:?}"
    );
    assert!(
        response.to_ascii_lowercase().contains("audio/flac"),
        "expected an audio/flac content type, got: {response:?}"
    );
    assert!(
        response.contains("fLaC-header"),
        "the header must be replayed to a joining consumer, got: {response:?}"
    );
}
