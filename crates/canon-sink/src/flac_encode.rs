//! PCM→FLAC encoder tap (yak canon-dfdd).
//!
//! This is the seam between the realtime mix and the LAN stream server: the engine pushes
//! interleaved `f32` frames at us through [`canon_core::PcmSink`], and we hand the
//! [`stream_server`](crate::stream_server) already-encoded FLAC — a cached header plus a
//! sequence of complete frames. The Chromecast sink (a later yak) is the thing that will
//! actually drive PCM in here; this module only cares about turning samples into FLAC and
//! never about *who* is listening.
//!
//! ## Why the header is precomputed once
//!
//! [`StreamBroadcaster`] replays a single cached header to every subscriber, new or
//! reconnecting (see its module docs on the tideway headerless-ring bug). So the header must
//! exist *before* the first frame and never change under a live subscriber. We build it in
//! [`FlacTap::new`] — `fLaC` magic + a last-block STREAMINFO with `total_samples = 0`
//! (unknown, because a live stream has no end) — and set it on the broadcaster there and then.
//! [`FlacTap::header`] hands back the same cached bytes for anyone who needs the exact blob a
//! decoder will initialise from (the tests, the Cast media description).
//!
//! ## Why this tap does not pace or resample
//!
//! Pacing is the engine's job: it delivers frames in realtime, and we encode whatever we are
//! given the instant a full [`BLOCK_SIZE`]-frame block has accumulated. We never sleep, never
//! resample, and never rate-limit — a block in is (eventually) a frame out. [`submit`] is
//! called from the realtime path, so it only ever buffers, converts, encodes, and pushes; it
//! must not block, and it doesn't.
//!
//! ## The construction spec is authoritative
//!
//! The sample rate, channel count, and bit depth are fixed at construction because they are
//! baked into the STREAMINFO header a decoder has already been handed. If a later [`submit`]
//! arrives claiming a *different* `sample_rate`/`channels`, honouring it would silently
//! contradict that header and corrupt the stream. So the per-call values are ignored and the
//! constructed spec wins: a mismatch is the caller violating the one-format-per-session
//! contract, not something we can encode correctly.
//!
//! [`submit`]: FlacTap::submit
//! [`StreamBroadcaster`]: crate::stream_server::StreamBroadcaster

use bytes::Bytes;
use canon_core::{Error, PcmSink, Result};
use flacenc::bitsink::ByteSink;
use flacenc::component::{BitRepr, StreamInfo};
use flacenc::config;
use flacenc::encode_fixed_size_frame;
use flacenc::error::{Verified, Verify};
use flacenc::source::{Fill, FrameBuf};

use crate::stream_server::StreamBroadcaster;

/// Frames per FLAC block. The FLAC reference default; large enough that per-frame overhead is
/// negligible, small enough that live latency stays a fraction of a second at any sane rate.
const BLOCK_SIZE: usize = 4096;

/// Turns realtime interleaved `f32` PCM into a live FLAC stream feeding a [`StreamBroadcaster`].
///
/// Construct one per playback session with [`new`](Self::new); feed it with the
/// [`PcmSink`] impl; call [`flush`](Self::flush) at stop so a partial trailing block is not
/// stranded. Everything about the FLAC format (rate, channels, bit depth) is fixed at
/// construction and encoded into the header the broadcaster replays — see the module docs.
pub struct FlacTap {
    /// Where encoded FLAC goes. The header is set on it once in `new`; frames are `push`ed.
    broadcaster: StreamBroadcaster,
    /// Verified encoder config. The block size in here is irrelevant —
    /// [`encode_fixed_size_frame`] takes the block size from the [`FrameBuf`] — but the rest
    /// (stereo/subframe coding choices) drives how each frame is encoded.
    config: Verified<config::Encoder>,
    /// The stream metadata every frame is encoded against. Consistent with the header blob.
    stream_info: StreamInfo,
    /// Reusable per-block buffer, allocated once at `BLOCK_SIZE`. A short final block reuses it
    /// too: `fill_interleaved` sets its filled size to the partial count and the encoder takes
    /// the block size from that, so the trailing frame is correctly shorter.
    framebuf: FrameBuf,
    /// The cached `fLaC` + STREAMINFO header, handed to the broadcaster and out via `header`.
    header: Bytes,
    /// Channel count from construction; the deinterleave width and block-completion divisor.
    channels: usize,
    /// Bit depth from construction; sets the f32→i32 scale factor.
    bits_per_sample: usize,
    /// Interleaved i32 samples not yet emitted, at construction bit depth. Drains a full block
    /// at a time in `submit`; the remainder rides here until the next block completes or `flush`.
    pending: Vec<i32>,
    /// FLAC fixed-blocksize frame number, incremented per emitted frame. Carried in each frame
    /// header so a decoder can order/seek; must stay strictly increasing across the session.
    frame_number: usize,
}

impl FlacTap {
    /// Build a tap for a fixed FLAC format and prime the broadcaster's header.
    ///
    /// Precomputes the `fLaC` magic + a last-block STREAMINFO metadata block (with
    /// `total_samples = 0`, the "unknown length" value a live stream needs) and calls
    /// [`StreamBroadcaster::set_header`] once, so every current and future subscriber can be
    /// initialised before the first audio frame.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Sink`] if the format is outside FLAC's valid ranges (bad channel count,
    /// bit depth, or sample rate) or the header fails to serialise.
    pub fn new(
        broadcaster: StreamBroadcaster,
        sample_rate: u32,
        channels: u16,
        bits_per_sample: u16,
    ) -> Result<Self> {
        let channels = channels as usize;
        let bits_per_sample = bits_per_sample as usize;

        let mut stream_info = StreamInfo::new(sample_rate as usize, channels, bits_per_sample)
            .map_err(|e| Error::Sink(format!("flac streaminfo: {e}")))?;
        // Live stream: length is unknown. This is the STREAMINFO "unknown" sentinel.
        stream_info.set_total_samples(0);
        // Fixed block size: min == max tells a decoder every frame but the last is BLOCK_SIZE,
        // which is exactly what a fixed-blocksize encoder emits.
        stream_info
            .set_block_sizes(BLOCK_SIZE, BLOCK_SIZE)
            .map_err(|e| Error::Sink(format!("flac block sizes: {e}")))?;

        let config = config::Encoder::default()
            .into_verified()
            .map_err(|(_, e)| Error::Sink(format!("flac config: {e}")))?;

        let framebuf = FrameBuf::with_size(channels, BLOCK_SIZE)
            .map_err(|e| Error::Sink(format!("flac framebuf: {e}")))?;

        let header = build_header(&stream_info)?;
        broadcaster.set_header(header.clone());

        Ok(Self {
            broadcaster,
            config,
            stream_info,
            framebuf,
            header,
            channels,
            bits_per_sample,
            pending: Vec::with_capacity(BLOCK_SIZE * channels),
            frame_number: 0,
        })
    }

    /// The cached decoder-init blob: `fLaC` magic + last-block STREAMINFO, 42 bytes. This is the
    /// exact byte sequence the broadcaster replays first to every subscriber. Cheap `Bytes` clone.
    #[must_use]
    pub fn header(&self) -> Bytes {
        self.header.clone()
    }

    /// Encode any buffered partial block as a final, shorter frame.
    ///
    /// A stop can land with fewer than [`BLOCK_SIZE`] frames buffered. Rather than strand them
    /// (silence at the tail) or wait forever for a block that will never complete, we emit them
    /// as a single short frame: `fill_interleaved` sets the frame buffer's filled size to the
    /// partial count and the encoder derives the (shorter) block size from that. FLAC allows a
    /// final frame shorter than the stream's block size, so this is a valid end to the stream
    /// — no silence padding needed. A stray sub-frame tail (fewer samples than one interleaved
    /// frame, only possible from a malformed submit) is dropped rather than encoded skewed.
    pub fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        // Whole interleaved frames only; discard any sub-frame remainder (malformed input).
        let usable = self.pending.len() - self.pending.len() % self.channels;
        if usable > 0 {
            let block: Vec<i32> = self.pending.drain(..usable).collect();
            if let Err(e) = self.encode_and_push(&block) {
                tracing::error!("canon-sink flac flush dropped final block: {e}");
            }
        }
        self.pending.clear();
    }

    /// Encode one interleaved block and push it to the broadcaster. `samples` must be whole
    /// interleaved frames; its length in frames becomes the encoded block size.
    fn encode_and_push(&mut self, samples: &[i32]) -> Result<()> {
        self.framebuf
            .fill_interleaved(samples)
            .map_err(|e| Error::Sink(format!("flac framebuf fill: {e}")))?;
        let frame = encode_fixed_size_frame(
            &self.config,
            &self.framebuf,
            self.frame_number,
            &self.stream_info,
        )
        .map_err(|e| Error::Sink(format!("flac encode: {e}")))?;

        let mut sink = ByteSink::new();
        frame
            .write(&mut sink)
            .map_err(|e| Error::Sink(format!("flac frame serialize: {e}")))?;
        self.broadcaster.push(Bytes::from(sink.into_inner()));
        self.frame_number += 1;
        Ok(())
    }
}

impl PcmSink for FlacTap {
    /// Buffer interleaved `f32`, emitting one FLAC frame per completed [`BLOCK_SIZE`] block.
    ///
    /// The per-call `sample_rate`/`channels` are intentionally ignored: the format is fixed at
    /// construction and baked into the header (see the module docs). Never blocks — it only
    /// converts, buffers, encodes ready blocks, and pushes.
    fn submit(&mut self, frames: &[f32], _sample_rate: u32, _channels: u16) {
        // FLAC is integer PCM. Scale the [-1.0, 1.0] float range to the full signed range of
        // the configured bit depth; clamp first so an out-of-range input can't overflow the
        // integer or exceed the sample range the encoder verifies against.
        let scale = ((1i64 << (self.bits_per_sample - 1)) - 1) as f32;
        self.pending.extend(
            frames
                .iter()
                .map(|&x| (x.clamp(-1.0, 1.0) * scale).round() as i32),
        );

        let block_len = BLOCK_SIZE * self.channels;
        while self.pending.len() >= block_len {
            let block: Vec<i32> = self.pending.drain(..block_len).collect();
            if let Err(e) = self.encode_and_push(&block) {
                // A validated, in-range block should not fail to encode; surface it loudly
                // rather than silently corrupting the frame sequence with a gap.
                tracing::error!("canon-sink flac encode dropped a block: {e}");
            }
        }
    }
}

/// Serialise the FLAC stream header: `fLaC` magic + a single last-block STREAMINFO metadata
/// block. The STREAMINFO *body* is produced by flacenc's own [`BitRepr`] (so its byte layout is
/// exactly what the library's decoder expects); we frame it with the metadata-block header by
/// hand — the last-block bit (`0x80`) OR'd with the STREAMINFO type tag (`0`), then the 24-bit
/// big-endian body length — because flacenc's `MetadataBlock` wrapper is crate-private. This
/// mirrors that wrapper's `write` byte-for-byte.
fn build_header(stream_info: &StreamInfo) -> Result<Bytes> {
    let mut body = ByteSink::new();
    stream_info
        .write(&mut body)
        .map_err(|e| Error::Sink(format!("flac streaminfo serialize: {e}")))?;
    let body = body.into_inner();

    let len = u32::try_from(body.len())
        .map_err(|_| Error::Sink("flac streaminfo too large".to_string()))?;

    let mut header = Vec::with_capacity(4 + 4 + body.len());
    header.extend_from_slice(b"fLaC");
    // Metadata-block header: 0x80 (last block) | 0x00 (STREAMINFO), then 24-bit body length.
    header.push(0x80);
    header.extend_from_slice(&len.to_be_bytes()[1..]);
    header.extend_from_slice(&body);
    Ok(Bytes::from(header))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::f32::consts::TAU;
    use std::pin::pin;
    use std::time::Duration;

    use tokio::time::timeout;
    use tokio_stream::StreamExt;

    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;
    const BITS: u16 = 16;

    /// A stereo sine, `frames` interleaved frames, amplitude 0.5 — non-constant so the encoder
    /// exercises a real subframe rather than a degenerate constant one.
    fn sine(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let s = (i as f32 * TAU * 440.0 / SAMPLE_RATE as f32).sin() * 0.5;
                [s, s]
            })
            .collect()
    }

    fn tap() -> (StreamBroadcaster, FlacTap) {
        let bc = StreamBroadcaster::new(64);
        let tap = FlacTap::new(bc.clone(), SAMPLE_RATE, CHANNELS, BITS).expect("valid format");
        (bc, tap)
    }

    /// The header is the exact 42-byte decoder-init blob: magic + a last-block STREAMINFO whose
    /// 34-byte body is announced by a 24-bit length of 0x22.
    #[test]
    fn header_is_magic_plus_last_block_streaminfo() {
        let (_bc, tap) = tap();
        let h = tap.header();

        assert!(h.starts_with(b"fLaC"), "starts with the FLAC magic");
        assert_eq!(h.len(), 42, "4 magic + 4 block header + 34 STREAMINFO body");
        assert_eq!(h[4] & 0x80, 0x80, "last-metadata-block bit is set");
        assert_eq!(h[4] & 0x7f, 0x00, "block type is STREAMINFO (0)");
        assert_eq!(&h[5..8], [0x00, 0x00, 0x22].as_slice(), "body length is 34");
    }

    /// A full block's worth of PCM emits exactly one frame; the broadcaster replays the header
    /// first, then the frame, whose first two bytes are a fixed-blocksize FLAC frame sync.
    #[tokio::test]
    async fn one_full_block_emits_one_frame_after_the_header() {
        let (bc, mut tap) = tap();
        let mut sub = pin!(bc.subscribe());

        tap.submit(&sine(BLOCK_SIZE), SAMPLE_RATE, CHANNELS);

        let first = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("header arrives")
            .expect("stream is live");
        assert_eq!(&first[..], &tap.header()[..], "header is replayed first");

        let frame = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("frame arrives")
            .expect("stream is live");
        assert_eq!(frame[0], 0xFF, "frame sync byte 1");
        assert_eq!(frame[1] & 0xFC, 0xF8, "frame sync byte 2 (fixed blocksize)");

        // Exactly one frame: nothing more is waiting.
        assert!(
            timeout(Duration::from_millis(100), sub.next())
                .await
                .is_err(),
            "no second frame from a single block"
        );
    }

    /// Less than a block buffers silently: the header is replayed, but no frame follows until a
    /// block completes.
    #[tokio::test]
    async fn a_partial_block_emits_no_frame() {
        let (bc, mut tap) = tap();
        let mut sub = pin!(bc.subscribe());

        tap.submit(&sine(BLOCK_SIZE / 4), SAMPLE_RATE, CHANNELS);

        let first = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("header arrives")
            .expect("stream is live");
        assert_eq!(&first[..], &tap.header()[..]);

        assert!(
            timeout(Duration::from_millis(100), sub.next())
                .await
                .is_err(),
            "a partial block must not emit a frame"
        );
    }

    /// `flush` emits the buffered partial block as a final short frame rather than stranding it.
    #[tokio::test]
    async fn flush_emits_the_buffered_partial_block() {
        let (bc, mut tap) = tap();
        let mut sub = pin!(bc.subscribe());

        tap.submit(&sine(500), SAMPLE_RATE, CHANNELS); // < BLOCK_SIZE: nothing emitted yet
        tap.flush();

        let first = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("header arrives")
            .expect("stream is live");
        assert_eq!(&first[..], &tap.header()[..]);

        let frame = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("flushed frame arrives")
            .expect("stream is live");
        assert_eq!(frame[0], 0xFF, "frame sync byte 1");
        assert_eq!(frame[1] & 0xFC, 0xF8, "frame sync byte 2");
    }

    /// Two full blocks in one submit produce two frames, with strictly increasing frame numbers
    /// (verified indirectly: both are valid frames and encoding the second didn't error out).
    #[tokio::test]
    async fn multiple_blocks_emit_multiple_frames() {
        let (bc, mut tap) = tap();
        let mut sub = pin!(bc.subscribe());

        tap.submit(&sine(BLOCK_SIZE * 2), SAMPLE_RATE, CHANNELS);

        let header = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("header arrives")
            .expect("stream is live");
        assert_eq!(&header[..], &tap.header()[..]);

        for n in 0..2 {
            let frame = timeout(Duration::from_millis(200), sub.next())
                .await
                .expect("frame arrives")
                .unwrap_or_else(|| panic!("frame {n} present"));
            assert_eq!(frame[0], 0xFF, "frame {n} sync byte 1");
            assert_eq!(frame[1] & 0xFC, 0xF8, "frame {n} sync byte 2");
        }
    }

    /// A mismatched per-call format is ignored: the constructed spec is authoritative, so a
    /// block still completes and encodes cleanly against the header the decoder was given.
    #[tokio::test]
    async fn submit_ignores_mismatched_format_and_keeps_going() {
        let (bc, mut tap) = tap();
        let mut sub = pin!(bc.subscribe());

        // Wrong rate and channel count on the call — must not change how we encode.
        tap.submit(&sine(BLOCK_SIZE), 44_100, 1);

        let header = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("header arrives")
            .expect("stream is live");
        assert_eq!(&header[..], &tap.header()[..]);

        let frame = timeout(Duration::from_millis(200), sub.next())
            .await
            .expect("frame arrives")
            .expect("stream is live");
        assert_eq!(frame[0], 0xFF);
        assert_eq!(frame[1] & 0xFC, 0xF8);
    }
}
