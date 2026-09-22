//! canon-sink — network renderers and the machinery to find and feed them.
//!
//! Responsibilities (see yak canon-7718 and its children):
//! * **Session lifecycle** (canon-cf48): the [`canon_core::Sink`] trait plus the
//!   [`canon_core::OutputRoute`] RAII un-silence primitive live in `canon-core`; the
//!   concrete renderer impls (Cast, DLNA) land here on top of them.
//! * **Resilient discovery** ([`discovery`], canon-ea5d): a supervised, self-healing
//!   service that enumerates real LAN interfaces and *excludes tunnels* (utun/VPN), pins
//!   multicast egress, and rebuilds sockets + rejoins groups on sleep/wake (tideway
//!   tide-6fd0/8f5b).
//! * **LAN stream server** ([`stream_server`], canon-21f7): serves FLAC to renderers,
//!   always replaying the header to a (re)joining consumer, with bounded per-reader
//!   backpressure.
//! * **Cast + DLNA control** (canon-dde4, canon-685a): AVTransport SOAP / Cast app
//!   framework, feeding device state *back* into the player state machine.

pub mod discovery;
pub mod stream_server;

pub use discovery::{DiscoveredDevice, DiscoveryService, Iface, usable_interfaces};
pub use stream_server::{STREAM_PATH, StreamBroadcaster, router, serve, spawn};
