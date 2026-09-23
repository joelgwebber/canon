//! # canon-core
//!
//! The service-agnostic heart of the canon music daemon: the entity/identity types,
//! the authoritative playback-state types, and the two seams every peripheral crate
//! plugs into — [`Source`] (bytes in) and [`Sink`] (audio out).
//!
//! Peripheral crates depend *inward* on this one only:
//!
//! ```text
//!   canon-tidal ─┐                         ┌─ canon-audio (decode/DSP/local out)
//!   (Source)     ├─▶  canon-core  ◀────────┤
//!   canon-library┘   (traits, state,       └─ canon-sink  (Sink impls + discovery)
//!   (MBID entities)   commands, ids)
//!                          ▲
//!                          └── canon-api (ws+json control + MCP)  ──▶ canon-daemon
//! ```
//!
//! ## Design directives encoded here
//! * **One authoritative playback state.** [`state`] — the realtime callback owns no
//!   clock-of-record; it advances a [`FrameClock`] while the control task derives
//!   position and emits seq-stamped [`PlayerSnapshot`]s.
//! * **Position has one authority per output.** [`position`] — local playback derives it
//!   from emitted frames, a network renderer reports its own and we reconcile to it.
//! * **Uniform outputs.** [`sink`] — local and network renderers share one trait;
//!   "which sink" is state, and un-silencing is RAII-bound so teardown can't leak.
//! * **Service-agnostic identity.** [`id`] — a canon-native [`EntityId`] handle with
//!   MusicBrainz MBIDs as the canonical cross-service identity (in `canon-library`).
//! * **One command vocabulary.** [`command`] — API and MCP both funnel into
//!   [`Command`]; events reconcile by `seq`.

mod command;
mod control;
mod error;
mod id;
mod player;
mod position;
mod session;
mod settings;
mod sink;
mod source;
mod state;
mod track;

pub use command::Command;
pub use control::ControlPlane;
pub use error::{Error, Result};
pub use id::{EntityId, Quality, Service, SourceRef};
pub use player::{Effect, EngineEvent, PlayerHandle, QueueSnapshot};
pub use position::{PositionDrive, Reconcile, RendererClock};
pub use session::{Account, DeviceCode, LoginStatus, ServiceSession};
pub use settings::{OutputMode, OutputSettings, Settings, SettingsStore};
pub use sink::{
    LoadId, LocalGate, OutputRoute, PcmSink, RendererEvent, RendererReport, RendererState,
    RouteGuard, Sink, SinkEndpoint, SinkId, SinkInfo, SinkKind,
};
pub use source::{MediaInput, ResolvedStream, Source, Sources};
pub use state::{FrameClock, PlaybackState, PlayerSnapshot, QueueView};
pub use track::{Codec, ReplayGain, StreamInfo, TrackMeta, TrackRef};
