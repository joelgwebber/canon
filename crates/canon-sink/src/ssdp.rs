//! SSDP search — how UPnP/DLNA renderers are found — sent from one pinned interface.
//!
//! This is written here rather than taken from `rupnp`/`ssdp-client` for the same reason the mDNS
//! source pins its interfaces: those send `M-SEARCH` from an unbound socket and let the route
//! table pick the egress, and (for eventing) guess the local address as "the first private IPv4".
//! After a VPN has come and gone, either guess can land on a dead tunnel — the tideway black hole
//! — and discovery goes quiet with no error. A search here is bound to an address on a chosen NIC
//! and pins multicast egress to it, so the query leaves where the renderers are and the unicast
//! replies come back to the same socket.
//!
//! `rupnp` is still used for what it does well: fetching and parsing the device description a
//! response points at, and SOAP actions.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use tokio::net::UdpSocket;

/// The SSDP multicast group and port (UPnP Device Architecture §1).
const SSDP_GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);

/// One device that answered a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SsdpHit {
    /// The device description URL (`LOCATION`).
    pub location: String,
    /// The unique service name (`USN`), which begins with the device's UDN.
    pub usn: String,
    /// How long the answer stays valid (`CACHE-CONTROL: max-age`), if the device said.
    pub max_age: Option<Duration>,
}

/// Search for `target` (an SSDP `ST`, e.g. a device-type URN) from the NIC with address `iface`,
/// collecting answers for `wait`. Responses are deduplicated by `USN`: devices commonly answer the
/// same search more than once.
///
/// # Errors
/// An I/O error if the socket can't be bound or pinned to `iface` — a NIC that went away between
/// enumeration and search, typically. No answers at all is `Ok(vec![])`.
pub(crate) async fn search(
    iface: Ipv4Addr,
    target: &str,
    wait: Duration,
) -> std::io::Result<Vec<SsdpHit>> {
    let socket = bind_pinned(iface)?;
    // MX is the window devices spread their replies over; ask for a little less than we wait so
    // the stragglers still land inside it.
    let mx = wait.as_secs().saturating_sub(1).clamp(1, 5);
    let request = format!(
        "M-SEARCH * HTTP/1.1\r\n\
         HOST: {SSDP_GROUP}\r\n\
         MAN: \"ssdp:discover\"\r\n\
         MX: {mx}\r\n\
         ST: {target}\r\n\
         \r\n"
    );
    // UDP is lossy and a sleepy renderer can miss one; a second query costs nothing.
    for _ in 0..2 {
        socket.send_to(request.as_bytes(), SSDP_GROUP).await?;
    }

    let mut hits: HashMap<String, SsdpHit> = HashMap::new();
    let mut buf = [0u8; 2048];
    let deadline = tokio::time::Instant::now() + wait;
    while let Ok(received) = tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
        let (len, _from) = received?;
        if let Some(hit) = parse_response(&String::from_utf8_lossy(&buf[..len])) {
            hits.entry(hit.usn.clone()).or_insert(hit);
        }
    }
    Ok(hits.into_values().collect())
}

/// A UDP socket bound to `iface` with multicast egress pinned to it.
fn bind_pinned(iface: Ipv4Addr) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.bind(&SockAddr::from(SocketAddr::from(SocketAddrV4::new(
        iface, 0,
    ))))?;
    socket.set_multicast_if_v4(&iface)?;
    // Stay on the local segment; renderers are never routed hops away.
    socket.set_multicast_ttl_v4(2)?;
    socket.set_nonblocking(true)?;
    UdpSocket::from_std(socket.into())
}

/// Parse one search response. `None` for anything that isn't a `200` carrying a `LOCATION` and a
/// `USN` — a malformed or unrelated datagram is skipped, not an error.
pub(crate) fn parse_response(text: &str) -> Option<SsdpHit> {
    let mut lines = text.split("\r\n");
    let status = lines.next()?;
    if !status.starts_with("HTTP/1.1 200") && !status.starts_with("HTTP/1.0 200") {
        return None;
    }
    let mut location = None;
    let mut usn = None;
    let mut max_age = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "location" => location = Some(value.to_string()),
            "usn" => usn = Some(value.to_string()),
            "cache-control" => max_age = parse_max_age(value),
            _ => {}
        }
    }
    Some(SsdpHit {
        location: location?,
        usn: usn?,
        max_age,
    })
}

/// `max-age=1800` (possibly among other directives, possibly spaced) → 1800 s.
fn parse_max_age(cache_control: &str) -> Option<Duration> {
    cache_control.split(',').find_map(|directive| {
        let (name, value) = directive.split_once('=')?;
        (name.trim().eq_ignore_ascii_case("max-age"))
            .then(|| value.trim().parse().ok().map(Duration::from_secs))
            .flatten()
    })
}

/// The device's UDN (`uuid:…`) from a `USN`, which is either the bare UDN or `UDN::type`.
pub(crate) fn udn_of(usn: &str) -> &str {
    usn.split("::").next().unwrap_or(usn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_typical_response() {
        let text = "HTTP/1.1 200 OK\r\n\
            CACHE-CONTROL: max-age = 1800\r\n\
            EXT:\r\n\
            Location: http://192.168.0.205:49152/description.xml\r\n\
            SERVER: Linux/4.9 UPnP/1.0 KEF/1.0\r\n\
            ST: urn:schemas-upnp-org:device:MediaRenderer:1\r\n\
            USN: uuid:5f9ec1b3-ed59-1900-4530-00a0dea4a6f4::urn:schemas-upnp-org:device:MediaRenderer:1\r\n\
            \r\n";
        let hit = parse_response(text).expect("a valid response");
        assert_eq!(hit.location, "http://192.168.0.205:49152/description.xml");
        assert_eq!(hit.max_age, Some(Duration::from_secs(1800)));
        assert_eq!(
            udn_of(&hit.usn),
            "uuid:5f9ec1b3-ed59-1900-4530-00a0dea4a6f4"
        );
    }

    #[test]
    fn ignores_anything_that_is_not_a_located_answer() {
        assert_eq!(
            parse_response("NOTIFY * HTTP/1.1\r\nLOCATION: x\r\nUSN: y\r\n"),
            None
        );
        assert_eq!(parse_response("HTTP/1.1 200 OK\r\nUSN: y\r\n"), None);
        assert_eq!(parse_response("garbage"), None);
    }

    #[test]
    fn max_age_is_found_among_other_directives() {
        assert_eq!(
            parse_max_age("no-cache=\"Ext\", max-age=900"),
            Some(Duration::from_secs(900))
        );
        assert_eq!(parse_max_age("no-cache"), None);
    }

    #[test]
    fn a_bare_udn_usn_is_its_own_udn() {
        assert_eq!(udn_of("uuid:abc"), "uuid:abc");
    }
}
