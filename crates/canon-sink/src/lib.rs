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
//! * **PCM→FLAC encoder tap** ([`flac_encode`], canon-dfdd): turns the engine's f32 PCM
//!   into a live FLAC stream feeding the [`stream_server`].
//! * **Cast control** ([`cast`], canon-dde4): connect + LOAD + MEDIA_STATUS fed *back*
//!   into the player state machine. DLNA (canon-685a) lands alongside it later.

pub mod cast;
pub mod discovery;
pub mod flac_encode;
pub mod stream_server;

pub use discovery::{DiscoveredDevice, DiscoveryService, Iface, usable_interfaces};
pub use flac_encode::FlacTap;
pub use stream_server::{STREAM_PATH, StreamBroadcaster, router, serve, spawn};
