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

use canon_core as _; // implemented against next.
