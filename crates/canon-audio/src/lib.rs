//! canon-audio — decode → DSP → output, plus the realtime plumbing.
//!
//! Responsibilities (see yak canon-b192 and its children):
//! * **The engine** ([`engine`]): [`AudioPlayer`] decodes a [`canon_core::MediaInput`] with
//!   Symphonia and feeds one [`Output`] — the local device (cpal, with device-loss and
//!   sleep/wake recovery), or a network renderer's [`canon_core::PcmSink`] paced to realtime —
//!   joining a prepared successor on without a gap where the format allows.
//! * **Ring + callback** (canon-8629): one lock-free SPSC ring of interleaved frames; the output
//!   callback is allocation/lock/syscall-free and only advances the [`canon_core::FrameClock`].
//! * **Resampling** ([`resample`]): when the device can't run at the source rate.
//! * **DSP** (canon-caae, not yet built): ReplayGain → EQ → crossfeed → volume, with crossfade at
//!   joins; realtime-safe by construction.
//!
//! ## The fMP4 decode check (canon-c4c3)
//!
//! [`decode`] is the seam that de-risked Tidal's container: decoding FLAC and AAC inside
//! *fragmented* MP4. [`decode::decode`] runs a whole input through Symphonia and summarises it,
//! and `tests/decode_fmp4.rs` keeps that assumption checked against synthesised fMP4 assets.

pub mod decode;
pub mod engine;
mod error;
pub mod resample;

pub use decode::{DecodeError, DecodeSummary, SeekableInput, decode};
pub use engine::{AudioPlayer, Output};
pub use error::PlayError;

// Re-exported so the decode seam's public surface (and its tests) can name the core
// types without a direct canon-core dependency.
pub use canon_core::{Codec, MediaInput};
