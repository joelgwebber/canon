//! canon-sink — network renderers and the machinery to find and feed them.
//!
//! Responsibilities (see yak canon-7718 and its children):
//! * **Renderer sessions** ([`renderer`], canon-583a): [`connect`] turns a discovered device
//!   into a `Box<dyn canon_core::Sink>` plus its [`RendererEvents`], whatever the protocol, and
//!   [`EdgeFilter`] is the one level-vs-edge rule every protocol's reports pass through.
//! * **Resilient discovery** ([`discovery`], canon-ea5d): a supervised, self-healing
//!   service that enumerates real LAN interfaces and *excludes tunnels* (utun/VPN), pins
//!   multicast egress, and rebuilds sockets + rejoins groups on sleep/wake (tideway
//!   tide-6fd0/8f5b).
//! * **LAN stream server** ([`stream_server`], canon-21f7): serves FLAC to renderers,
//!   always replaying the header to a (re)joining consumer, with bounded per-reader
//!   backpressure.
//! * **PCM→FLAC encoder tap** ([`flac_encode`], canon-dfdd): turns the engine's f32 PCM
//!   into a live FLAC stream feeding the [`stream_server`].
//! * **Cast control** ([`cast`], canon-dde4): connect + LOAD + MEDIA_STATUS classified into
//!   renderer events.
//! * **DLNA control** ([`dlna`], canon-685a): SSDP discovery ([`discovery`]) plus AVTransport over
//!   SOAP, with polled state classified into the same renderer events.

pub mod cast;
pub mod discovery;
pub mod dlna;
pub mod flac_encode;
pub mod renderer;
mod ssdp;
pub mod stream_server;

pub use discovery::{
    DiscoveredDevice, DiscoveryService, Iface, host_interfaces, usable_interfaces,
};
pub use flac_encode::FlacTap;
pub use renderer::{EdgeFilter, RendererEvents, connect, outputs};
pub use stream_server::{StreamBroadcaster, StreamRoutes, router, serve, spawn};
