//! `canon cast` — the on-metal harness for the whole network-sink path (yak canon-dde4).
//!
//! Ties every network piece together in one command, so the live path can be exercised without a
//! running daemon or a client:
//!
//! ```text
//!   Tidal stream ──▶ decode ──▶ engine (Output::Network) ──▶ FlacTap ──▶ StreamBroadcaster
//!                                                                              │
//!                                            LAN FLAC server (chosen iface) ◀──┘
//!                                                     ▲ HTTP GET
//!                                            Chromecast receiver  ◀── LOAD(url, audio/flac, LIVE)
//!                                                     │
//!                                             MEDIA_STATUS ──▶ RendererEvent (printed here)
//! ```
//!
//! The device's reported status is echoed to the terminal, which is what makes an external
//! takeover (casting something else to the same speaker from a phone) visible as a real event
//! rather than a silent desync — the tideway failure this whole path is designed against.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use canon_core::{
    Codec, FrameClock, PcmSink, Quality, RendererEvent, RendererState, ServiceSession, Sink,
    SinkId, TrackMeta,
};
use canon_sink::cast::CastSink;
use canon_sink::{FlacTap, StreamRoutes};

use crate::{BoxError, build_tidal_session};

/// How long to wait for the discovery browse when resolving a device by name.
const DISCOVERY_WINDOW: Duration = Duration::from_secs(5);

pub async fn run(
    state_dir: &std::path::Path,
    track_id: &str,
    target: &str,
    quality: Quality,
) -> Result<(), BoxError> {
    let session = build_tidal_session(state_dir).await?;
    if !session.is_authenticated() {
        return Err("not logged in — run `canon login tidal --pkce` first".into());
    }

    // 1. Find the device: either an explicit host:port, or a discovered friendly name.
    let (sink_id, name, addr) = resolve_target(target).await?;
    println!("target: {name} at {addr}");

    // 2. Pick the LAN interface the renderer can reach us on, and bind the stream server there.
    //    Binding a specific interface (never 0.0.0.0) is the canon-21f7 contract.
    let local_ip = lan_ip()?;
    let routes = StreamRoutes::default();
    let (path, broadcaster) = routes.open();
    let (bound, _server) = canon_sink::spawn(SocketAddr::new(local_ip, 0), routes).await?;
    let url = format!("http://{bound}{path}");
    println!("serving stream at {url}");

    // 3. Resolve the Tidal stream and start the engine on the network output. The tap encodes to
    //    FLAC and primes the broadcaster's header before the device ever connects.
    println!("resolving track {track_id} …");
    let resolved = session.open_stream(track_id, quality).await?;
    let codec = resolved.info.codec;
    let hint = match codec {
        Codec::Flac => Some("flac".to_owned()),
        Codec::Aac | Codec::Alac => Some("m4a".to_owned()),
        _ => None,
    };
    let rate = resolved.info.sample_rate;
    let channels = resolved.info.channels;
    // FLAC is integer PCM; 16 bits is universally supported by Cast receivers.
    let tap = FlacTap::new(broadcaster, rate, channels, 16)?;
    println!("encoding {codec:?} {rate} Hz × {channels}ch → FLAC");

    let clock = Arc::new(FrameClock::new());
    let (events_tx, mut engine_rx) = tokio::sync::mpsc::unbounded_channel();
    let _audio = canon_audio::AudioPlayer::start(
        resolved.input,
        hint,
        Arc::clone(&clock),
        events_tx,
        0,
        canon_audio::Output::Network {
            sink: Box::new(tap) as Box<dyn PcmSink>,
            joins: false,
        },
    );

    // 4. Connect and LOAD. The receiver then pulls the URL above.
    println!("connecting to {name} …");
    let (sink, mut events) = CastSink::connect(sink_id, name.clone(), addr).await?;
    sink.load(&url, &TrackMeta::default())?;
    println!("LOAD issued; watching device status (ctrl-c to stop) …");

    // 5. Report what the *device* says, plus engine events, until it ends or is taken over.
    loop {
        tokio::select! {
            event = events.recv() => match event.map(|report| report.event) {
                Some(RendererEvent::State(RendererState::Playing)) => println!("[device] playing"),
                Some(RendererEvent::State(RendererState::Paused)) => println!("[device] paused"),
                Some(RendererEvent::State(RendererState::Buffering)) => {
                    println!("[device] buffering");
                }
                // The gap between these two numbers is the reason the player does not treat
                // frames fed as position: it is the receiver's buffer plus our pacing lead.
                Some(RendererEvent::Position(position)) => println!(
                    "[device] at {:.1}s (we have fed {:.1}s — {:+.1}s ahead)",
                    position.as_secs_f64(),
                    clock.position().as_secs_f64(),
                    clock.position().as_secs_f64() - position.as_secs_f64(),
                ),
                Some(RendererEvent::Ended) => {
                    println!("[device] ended");
                    break;
                }
                Some(RendererEvent::Superseded(why)) => {
                    println!("[device] TAKEOVER: {why}");
                    break;
                }
                Some(RendererEvent::Failed(why)) => {
                    println!("[device] FAILED: {why}");
                    break;
                }
                None => break,
            },
            engine = engine_rx.recv() => match engine {
                Some(canon_core::EngineEvent::Loaded { sample_rate, .. }) => {
                    // No player actor on this path, so this CLI owns the clock: rebase it
                    // here or frames never convert to a time.
                    clock.reset(sample_rate);
                    println!("[engine] feeding at {sample_rate} Hz");
                }
                Some(canon_core::EngineEvent::Failed(why)) => {
                    println!("[engine] FAILED: {why}");
                    break;
                }
                Some(_) => {}
                // The engine finished feeding; the device is still draining its buffer, so keep
                // watching device status rather than declaring completion here.
                None => {}
            },
            _ = tokio::signal::ctrl_c() => {
                println!("interrupted");
                break;
            }
        }
    }

    println!("stopping …");
    let _ = sink.stop();
    Ok(())
}

/// Resolve `target` to a device: an explicit `host:port`, or a friendly name looked up by browsing.
async fn resolve_target(target: &str) -> Result<(SinkId, String, SocketAddr), BoxError> {
    if let Ok(addr) = target.parse::<SocketAddr>() {
        return Ok((SinkId(target.to_string()), target.to_string(), addr));
    }
    if let Ok(ip) = target.parse::<IpAddr>() {
        let addr = SocketAddr::new(ip, 8009);
        return Ok((SinkId(target.to_string()), target.to_string(), addr));
    }

    println!("browsing for “{target}” …");
    let discovery = canon_sink::DiscoveryService::spawn()?;
    let deadline = tokio::time::Instant::now() + DISCOVERY_WINDOW;
    let mut devices = discovery.devices();
    loop {
        if let Some(found) = devices
            .borrow()
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(target))
            .cloned()
        {
            return Ok((found.id, found.name, found.addr));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "no renderer named “{target}” found; run `canon devices` to list them"
            )
            .into());
        }
        let _ = tokio::time::timeout(Duration::from_millis(250), devices.changed()).await;
    }
}

/// The LAN address a renderer can reach us on, chosen with the same interface filter discovery
/// uses (so tunnels/VPNs are excluded — a utun address would be unreachable from the speaker).
fn lan_ip() -> Result<IpAddr, BoxError> {
    let interfaces = if_addrs_v4()?;
    interfaces
        .first()
        .copied()
        .ok_or_else(|| "no usable LAN interface found".into())
}

/// IPv4 addresses of usable LAN interfaces, in discovery's preference order.
fn if_addrs_v4() -> Result<Vec<IpAddr>, BoxError> {
    let all = canon_sink::host_interfaces();
    Ok(canon_sink::usable_interfaces(&all)
        .into_iter()
        .map(|i| i.ip)
        .filter(IpAddr::is_ipv4)
        .collect())
}
