//! canon-audio — decode → DSP → local output, plus the realtime plumbing.
//!
//! Responsibilities (see yak canon-b192 and its children):
//! * **Decode/demux** (canon-c4c3): adapt a [`canon_core::MediaInput`] into a
//!   Symphonia `MediaSource` and decode FLAC/AAC. Fragmented-MP4 demux is the known
//!   risk; ffmpeg is the side-quest fallback if Symphonia can't handle Tidal's fMP4.
//! * **Ring + callback** (canon-8629): one lock-free SPSC ring of interleaved frames;
//!   the output callback is allocation/lock/syscall-free and only advances the
//!   [`canon_core::FrameClock`].
//! * **Local sink** (canon-940d): a cpal [`canon_core::Sink`]/[`canon_core::PcmSink`]
//!   with device-loss + sleep/wake recovery routed through the state machine.
//! * **DSP** (canon-caae): ReplayGain → EQ → crossfeed → volume, with equal-power
//!   crossfade at track boundaries; realtime-safe by construction.

//! ## Spike status (canon-c4c3)
//!
//! The [`decode`] module is the de-risking spike for fragmented-MP4 support: a
//! working Symphonia probe/decode seam ([`decode::decode`]) plus tests
//! (`tests/decode_fmp4.rs`) that establish, empirically, whether Symphonia can
//! demux+decode FLAC and AAC inside fragmented MP4. The seam consumes
//! `canon_core::MediaInput` directly.

pub mod decode;
pub mod output;

pub use decode::{DecodeError, DecodeSummary, SeekableInput, decode};
pub use output::{PlayError, PlayStats, play_blocking};

// Re-exported so the decode seam's public surface (and its tests) can name the core
// types without a direct canon-core dependency.
pub use canon_core::{Codec, MediaInput};
