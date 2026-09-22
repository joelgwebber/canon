//! canon-sink — network renderers and the machinery to find and feed them.
//!
//! Responsibilities (see yak canon-7718 and its children):
//! * **Session lifecycle** (canon-cf48): [`canon_core::Sink`] impls whose teardown /
//!   local-un-silence is RAII-bound, driven by a liveness signal — so a dead receiver
//!   can't wedge local output (tideway tide-4000.x).
//! * **Resilient discovery** (canon-ea5d): a supervised, self-healing service that
//!   enumerates real LAN interfaces and *excludes tunnels* (utun/VPN), pins multicast
//!   egress, and rebuilds sockets + rejoins groups on sleep/wake (tideway
//!   tide-6fd0/8f5b).
//! * **LAN stream server** (canon-21f7): serves FLAC to renderers, always replaying
//!   the header to a (re)joining consumer, with bounded per-reader backpressure.
//! * **Cast + DLNA control** (canon-dde4, canon-685a): AVTransport SOAP / Cast app
//!   framework, feeding device state *back* into the player state machine.

use canon_core as _; // implemented against next.
