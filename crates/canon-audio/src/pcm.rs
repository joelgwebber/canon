//! A track decoded to integer PCM at its own bit depth and sample rate (yak canon-040e).
//!
//! The engine decodes to `f32` for the output path. A server handing audio on to a player wants
//! the samples back as they were: [`PcmStream`] is the same decode, read out as `i32` at the
//! source's bit depth, ready for a FLAC encoder.
//!
//! The round trip through `f32` is exact. Symphonia widens an N-bit integer sample to `f32` by
//! dividing by 2^(N-1), a power of two, so up to 24 bits (what an `f32` mantissa holds) nothing
//! is rounded; multiplying by 2^(N-1) again recovers the integer. Sources with no integer
//! samples (AAC, Vorbis) have no bit depth to be true to, and come out at [`LOSSY_BIT_DEPTH`].

use std::time::Duration;

use canon_core::MediaInput;

use crate::engine::Decode;
use crate::error::PlayError;

/// The depth a source with no integer samples of its own (AAC, Vorbis, MP3) is served at.
pub const LOSSY_BIT_DEPTH: u8 = 16;

/// The deepest sample FLAC can carry that an `f32` still holds exactly.
const MAX_BIT_DEPTH: u8 = 24;

/// Interleaved integer PCM from one opened track, from where it starts to its end.
pub struct PcmStream {
    decode: Decode,
    bit_depth: u8,
    start_ms: u64,
}

impl PcmStream {
    /// Open `input` for decoding. With `seek`, decode from that point of a seekable input: the
    /// demuxer lands on the packet at or before it. `start_ms` is where the bytes begin in track
    /// time, as the source reported it ([`canon_core::ResolvedStream::start_ms`]); a seek
    /// overrides it with where the demuxer really landed. `bit_depth_hint` is the source's own
    /// word for the depth, used when the codec doesn't say.
    ///
    /// # Errors
    /// The input doesn't probe or decode, or a seek isn't possible.
    pub fn open(
        input: Box<dyn MediaInput>,
        extension_hint: Option<&str>,
        start_ms: u64,
        seek: Option<Duration>,
        bit_depth_hint: Option<u8>,
    ) -> Result<Self, PlayError> {
        let decode = Decode::open(input, extension_hint, seek)?;
        let bit_depth = decode
            .bits_per_sample
            .and_then(|bits| u8::try_from(bits).ok())
            .or(bit_depth_hint)
            .unwrap_or(LOSSY_BIT_DEPTH)
            .clamp(1, MAX_BIT_DEPTH);
        let start_ms = decode.started_at_ms.unwrap_or(start_ms);
        Ok(Self {
            decode,
            bit_depth,
            start_ms,
        })
    }

    /// Samples per second, per channel.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.decode.source_rate
    }

    #[must_use]
    pub fn channels(&self) -> u16 {
        self.decode.channels
    }

    /// Bits in each sample returned by [`next_chunk`](Self::next_chunk).
    #[must_use]
    pub fn bit_depth(&self) -> u8 {
        self.bit_depth
    }

    /// Where the first sample sits in track time, in milliseconds.
    #[must_use]
    pub fn start_ms(&self) -> u64 {
        self.start_ms
    }

    /// The next run of whole interleaved frames, or `None` at the end of the track.
    ///
    /// # Errors
    /// The demuxer or decoder failed for good.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<i32>>, PlayError> {
        let scale = (1i64 << (self.bit_depth - 1)) as f64;
        let (min, max) = (-(scale as i64), scale as i64 - 1);
        Ok(self.decode.next()?.map(|chunk| {
            chunk
                .into_iter()
                .map(|x| ((f64::from(x) * scale).round() as i64).clamp(min, max) as i32)
                .collect()
        }))
    }
}
