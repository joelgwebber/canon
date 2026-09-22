//! Decode/demux seam (yak canon-c4c3): adapt a seekable byte reader into a
//! Symphonia `MediaSourceStream`, probe the container, and run the packet →
//! decoder loop to completion.
//!
//! This is the spike deliverable for the single riskiest assumption in the audio
//! plan: **can pure-Rust Symphonia demux + decode _fragmented_ MP4 (fMP4)?** Tidal
//! ships FLAC and AAC inside fragmented MP4 (`moof`/`mdat` segments, an
//! `empty_moov` with an `mvex`, not a plain `moov`+`mdat` file). The tests in
//! `tests/decode_fmp4.rs` drive this entry point over synthesized fMP4 assets and
//! a non-fragmented control to answer that question empirically.
//!
//! ## Seam note: [`canon_core::MediaInput`]
//!
//! Symphonia's [`symphonia::core::io::MediaSource`] is `Read + Seek + Send + Sync`,
//! and `canon_core::MediaInput` carries exactly those bounds, so the production
//! `Box<dyn MediaInput>` a source resolves feeds straight into [`decode`]. There is
//! no blanket `MediaSource` impl for an arbitrary seekable reader (only `File`,
//! `Cursor`, and `ReadOnlySource`), so we wrap the reader in [`SeekableInput`], which
//! implements `MediaSource`.

use std::io::{Read, Seek, SeekFrom};

use canon_core::Codec;
use symphonia::core::codecs::audio::well_known::{CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC};
use symphonia::core::codecs::audio::{AudioCodecId, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

/// A seekable reader adapted into a Symphonia [`MediaSource`]. This is the concrete
/// boundary the production `Box<dyn canon_core::MediaInput>` flows through.
///
/// `byte_len` is captured once at construction by measuring the stream and restoring
/// the cursor to its start, so the demuxer gets an authoritative length (helpful for
/// fragmented streams where end-of-stream is otherwise only knowable by hitting EOF).
pub struct SeekableInput<R: Read + Seek + Send + Sync> {
    inner: R,
    byte_len: Option<u64>,
}

impl<R: Read + Seek + Send + Sync> SeekableInput<R> {
    /// Wrap a seekable reader, measuring its total length and rewinding to the
    /// start. The reader is expected to be positioned so that a full read yields
    /// the whole stream (typically position 0).
    pub fn new(mut inner: R) -> std::io::Result<Self> {
        let start = inner.stream_position()?;
        let end = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(start))?;
        Ok(Self {
            inner,
            byte_len: Some(end.saturating_sub(start)),
        })
    }
}

impl<R: Read + Seek + Send + Sync> Read for SeekableInput<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<R: Read + Seek + Send + Sync> Seek for SeekableInput<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for SeekableInput<R> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.byte_len
    }
}

/// Outcome of a probe+decode run: enough of a `StreamInfo`-shaped picture to prove
/// the container was demuxed and the codec bitstream decoded end to end.
#[derive(Debug, Clone)]
pub struct DecodeSummary {
    /// The codec the container advertised for the default audio track, mapped onto
    /// canon's [`Codec`] vocabulary.
    pub codec: Codec,
    /// Sample rate in Hz, from the decoded audio (falls back to codec params).
    pub sample_rate: u32,
    /// Channel count, from the decoded audio (falls back to codec params).
    pub channels: u16,
    /// Bit depth as advertised by the codec parameters, if known.
    pub bit_depth: Option<u8>,
    /// Total audio frames (per-channel sample count) decoded across every packet.
    pub frames_decoded: u64,
    /// Number of container packets that belonged to the selected track.
    pub packets: u64,
    /// Per-packet decode failures that were skipped (bitstream errors). Zero on a
    /// clean decode.
    pub decode_errors: u64,
}

/// What went wrong, kept granular so the spike can distinguish "container refused
/// to probe" from "probed but yielded no packets" from "codec choked".
#[derive(Debug)]
pub enum DecodeError {
    /// Wrapping the reader as a `MediaSource` failed (I/O during length probe).
    Io(std::io::Error),
    /// `get_probe().probe(..)` rejected the stream — no format reader matched.
    Probe(SymphoniaError),
    /// Probed fine, but the container exposed no audio track.
    NoAudioTrack,
    /// The audio track had no codec parameters to build a decoder from.
    MissingCodecParams,
    /// No decoder is registered for the track's codec.
    UnsupportedCodec(SymphoniaError),
    /// A fatal (non-skippable) error while pulling packets from the demuxer.
    Demux(SymphoniaError),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Io(e) => write!(f, "io error preparing media source: {e}"),
            DecodeError::Probe(e) => write!(f, "format probe failed: {e}"),
            DecodeError::NoAudioTrack => write!(f, "no audio track in container"),
            DecodeError::MissingCodecParams => write!(f, "audio track missing codec parameters"),
            DecodeError::UnsupportedCodec(e) => write!(f, "no decoder for codec: {e}"),
            DecodeError::Demux(e) => write!(f, "fatal demux error: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

fn map_codec(id: AudioCodecId) -> Codec {
    match id {
        CODEC_ID_FLAC => Codec::Flac,
        CODEC_ID_AAC => Codec::Aac,
        CODEC_ID_ALAC => Codec::Alac,
        _ => Codec::Other,
    }
}

/// Probe and fully decode a seekable audio stream, returning a [`DecodeSummary`].
///
/// This is the seam the audio pipeline will grow around: hand it anything
/// `Read + Seek + Send + Sync` (a `File`, an in-memory `Cursor`, or — in
/// production — the Tidal segment reader; see the module docs on the `Sync`
/// requirement) plus an optional file-extension hint, and it drives Symphonia's
/// probe → default-track → decode-loop pipeline to EOF.
///
/// It decodes **every** packet to prove the whole segment stream demuxes, not just
/// that the first fragment probes.
pub fn decode<R>(input: R, extension_hint: Option<&str>) -> Result<DecodeSummary, DecodeError>
where
    R: Read + Seek + Send + Sync + 'static,
{
    let source = SeekableInput::new(input).map_err(DecodeError::Io)?;
    let mss = MediaSourceStream::new(Box::new(source), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = extension_hint {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(DecodeError::Probe)?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or(DecodeError::NoAudioTrack)?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or(DecodeError::MissingCodecParams)?
        .clone();

    let codec = map_codec(audio_params.codec);
    // FLAC advertises bit depth via codec params; AAC is inherently float and leaves
    // this None — a correct absence, not a missing field.
    let bit_depth = audio_params
        .bits_per_sample
        .map(|b| b.min(u32::from(u8::MAX)) as u8);
    let mut sample_rate = audio_params.sample_rate.unwrap_or(0);
    let mut channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count() as u16)
        .unwrap_or(0);

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(DecodeError::UnsupportedCodec)?;

    let mut frames_decoded: u64 = 0;
    let mut packets: u64 = 0;
    let mut decode_errors: u64 = 0;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break, // clean end of stream
            // ResetRequired means the track set changed (chained streams); for a
            // single-track fMP4 it should not occur. Treat it as the end and stop
            // rather than pretend to reconfigure — the summary still reflects what
            // decoded up to that point.
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(DecodeError::Demux(e)),
        };

        if packet.track_id != track_id {
            continue;
        }
        packets += 1;

        match decoder.decode(&packet) {
            Ok(decoded) => {
                if sample_rate == 0 {
                    sample_rate = decoded.spec().rate();
                }
                if channels == 0 {
                    channels = decoded.spec().channels().count() as u16;
                }
                frames_decoded += decoded.frames() as u64;
            }
            // Per-packet bitstream hiccups are skippable per Symphonia's decode-loop
            // contract; count them so the verdict can distinguish a clean decode
            // from a lossy one.
            Err(SymphoniaError::IoError(_)) | Err(SymphoniaError::DecodeError(_)) => {
                decode_errors += 1;
            }
            Err(e) => return Err(DecodeError::Demux(e)),
        }
    }

    Ok(DecodeSummary {
        codec,
        sample_rate,
        channels,
        bit_depth,
        frames_decoded,
        packets,
        decode_errors,
    })
}
