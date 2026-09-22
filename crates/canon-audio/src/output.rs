//! Local audio output: decode → a lock-free ring → the cpal realtime callback
//! (yaks canon-8629 + canon-940d).
//!
//! The architecture is the one the plan calls for, and the reason canon is native:
//! * A single **SPSC ring** ([`rtrb`]) of interleaved `f32` frames sits between the
//!   decode thread (producer) and cpal's realtime callback (consumer). It's sized in
//!   *time* (~500 ms), not chunk count.
//! * The **callback never blocks, allocates, locks, or syscalls**: it pops what's there,
//!   pads any shortfall with silence, and advances the shared [`FrameClock`] by the
//!   frames it actually pulled — so a decode hiccup shows up as the position briefly
//!   pausing, never as a frozen-but-"playing" emitter (the tideway lesson, in the clock).
//! * The producer **parks on a full ring** (backpressure), so a fast decoder can't
//!   outrun the device.
//!
//! This v1 is a blocking driver (`play_blocking`) good enough to prove sound end to end.
//! Device-loss / sleep-wake recovery and DSP are the rest of canon-940d / canon-caae.

use std::sync::Arc;
use std::time::Duration;

use canon_core::{FrameClock, MediaInput};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::decode::SeekableInput;

/// Target ring depth, in milliseconds of audio. Small is fine — there's no GIL to fight,
/// so latency stays low while still absorbing decode jitter.
const RING_MILLIS: u32 = 500;

/// How long to let the device play out its own buffer after the ring drains.
const DRAIN_TAIL: Duration = Duration::from_millis(250);

/// What a completed playback run produced — enough to confirm the whole path ran.
#[derive(Debug, Clone, Copy)]
pub struct PlayStats {
    pub sample_rate: u32,
    pub channels: u16,
    /// Total audio frames (per-channel) decoded and handed to the ring.
    pub frames_played: u64,
}

/// Everything that can go wrong opening or running the local output path.
#[derive(Debug)]
pub enum PlayError {
    Io(std::io::Error),
    Decode(String),
    /// No default output device is available.
    NoDevice,
    /// The device can't serve the source format and canon doesn't resample yet.
    UnsupportedFormat(String),
    /// cpal failed to build or start the stream.
    Device(String),
}

impl std::fmt::Display for PlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlayError::Io(e) => write!(f, "io: {e}"),
            PlayError::Decode(e) => write!(f, "decode: {e}"),
            PlayError::NoDevice => write!(f, "no default audio output device"),
            PlayError::UnsupportedFormat(e) => write!(f, "unsupported output format: {e}"),
            PlayError::Device(e) => write!(f, "audio device: {e}"),
        }
    }
}

impl std::error::Error for PlayError {}

/// Decode `input` and play it on the default output device, blocking until the track
/// finishes. `clock` is advanced by the realtime callback so a caller can observe
/// position exactly as the player state core does.
///
/// The source sample rate/channel count are learned from the first decoded packet, then
/// the device is opened to match (no resampling yet — an unsupported rate is a clear
/// error, not silent wrong-speed playback).
pub fn play_blocking(
    input: Box<dyn MediaInput>,
    extension_hint: Option<&str>,
    clock: Arc<FrameClock>,
) -> Result<PlayStats, PlayError> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let source = SeekableInput::new(input).map_err(PlayError::Io)?;
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
        .map_err(|e| PlayError::Decode(format!("probe: {e}")))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| PlayError::Decode("no audio track".into()))?;
    let track_id = track.id;
    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| PlayError::Decode("no codec params".into()))?
        .clone();

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| PlayError::Decode(format!("no decoder: {e}")))?;

    // Decode the first audio packet up front to learn the concrete spec (rate/channels),
    // which we need before we can open the device.
    let mut scratch: Vec<f32> = Vec::new();
    let (sample_rate, channels) = loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => return Err(PlayError::Decode("stream had no audio".into())),
            Err(e) => return Err(PlayError::Decode(format!("demux: {e}"))),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                let rate = decoded.spec().rate();
                let ch = decoded.spec().channels().count() as u16;
                decoded.copy_to_vec_interleaved(&mut scratch);
                break (rate, ch);
            }
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => return Err(PlayError::Decode(format!("decode: {e}"))),
        }
    };
    if sample_rate == 0 || channels == 0 {
        return Err(PlayError::Decode("stream reported no rate/channels".into()));
    }

    // Open the device to match the source. The frame clock is rebased here.
    let config = pick_output_config(sample_rate, channels)?;
    clock.reset(sample_rate);

    let capacity = (sample_rate as usize * channels as usize * RING_MILLIS as usize) / 1000;
    let (mut producer, mut consumer) =
        rtrb::RingBuffer::<f32>::new(capacity.max(channels as usize));

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(PlayError::NoDevice)?;
    let callback_clock = Arc::clone(&clock);
    let channels_usize = channels as usize;
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Realtime callback: pop what's available, pad the rest with silence,
                // and advance the clock only by the frames actually delivered.
                let mut filled = 0usize;
                for slot in data.iter_mut() {
                    match consumer.pop() {
                        Ok(sample) => {
                            *slot = sample;
                            filled += 1;
                        }
                        Err(_) => *slot = 0.0,
                    }
                }
                callback_clock.advance((filled / channels_usize) as u64);
            },
            |err| eprintln!("[audio] output stream error: {err}"),
            None,
        )
        .map_err(|e| PlayError::Device(format!("build stream: {e}")))?;
    stream
        .play()
        .map_err(|e| PlayError::Device(format!("play: {e}")))?;

    // Push the first packet's samples, then decode the rest, parking when the ring fills.
    let mut frames_played: u64 = 0;
    push_all(&mut producer, &scratch);
    frames_played += (scratch.len() / channels_usize) as u64;

    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => return Err(PlayError::Decode(format!("demux: {e}"))),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => {
                decoded.copy_to_vec_interleaved(&mut scratch);
                push_all(&mut producer, &scratch);
                frames_played += (scratch.len() / channels_usize) as u64;
            }
            Err(symphonia::core::errors::Error::DecodeError(_))
            | Err(symphonia::core::errors::Error::IoError(_)) => continue,
            Err(e) => return Err(PlayError::Decode(format!("decode: {e}"))),
        }
    }

    // Wait for the callback to drain the ring, then let the device play its tail out.
    while producer.slots() < capacity.max(channels as usize) {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(DRAIN_TAIL);
    drop(stream);

    Ok(PlayStats {
        sample_rate,
        channels,
        frames_played,
    })
}

/// Push every sample into the ring, parking briefly whenever it's full (backpressure).
fn push_all(producer: &mut rtrb::Producer<f32>, samples: &[f32]) {
    for &sample in samples {
        loop {
            match producer.push(sample) {
                Ok(()) => break,
                Err(rtrb::PushError::Full(_)) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
}

/// Find an `f32` output config on the default device that matches the source rate and
/// channel count exactly. No match is an honest error rather than resampled/wrong-speed
/// audio (resampling is future work).
pub(crate) fn pick_output_config(
    sample_rate: u32,
    channels: u16,
) -> Result<cpal::StreamConfig, PlayError> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(PlayError::NoDevice)?;
    let target = cpal::SampleRate(sample_rate);

    let supported = device
        .supported_output_configs()
        .map_err(|e| PlayError::Device(format!("query configs: {e}")))?;

    for range in supported {
        if range.channels() == channels
            && range.sample_format() == cpal::SampleFormat::F32
            && range.min_sample_rate() <= target
            && target <= range.max_sample_rate()
        {
            return Ok(range.with_sample_rate(target).config());
        }
    }
    Err(PlayError::UnsupportedFormat(format!(
        "default device has no f32 config for {sample_rate} Hz / {channels} ch (resampling not yet implemented)"
    )))
}
