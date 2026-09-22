//! A controllable, streaming audio player (yaks canon-940d + canon-a7d6).
//!
//! Where [`crate::output::play_blocking`] runs a whole file to completion, this is the
//! engine the daemon drives from the control plane: [`AudioPlayer::start`] kicks off
//! decode → ring → cpal in a background thread and returns immediately, then
//! [`pause`](AudioPlayer::pause)/[`resume`](AudioPlayer::resume)/[`stop`](AudioPlayer::stop)
//! and volume changes take effect live. Progress is reported back as
//! [`EngineEvent`]s (Loaded/Ended/Failed) so the player state actor stays authoritative.
//!
//! It consumes any [`MediaInput`] as a *forward* stream (no seeking assumed), so the
//! same engine plays a local file and canon-tidal's streaming segment reader.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use canon_core::{EngineEvent, FrameClock, MediaInput};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc::UnboundedSender;

/// Live control state shared between the API/daemon, the decode thread, and the realtime
/// callback. All lock-free so the callback never blocks.
struct Controls {
    paused: AtomicBool,
    stopped: AtomicBool,
    /// Playback gain in [0, 1], stored as `f32` bits.
    volume: AtomicU32,
    muted: AtomicBool,
}

impl Controls {
    fn new() -> Self {
        Self {
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
        }
    }
    fn gain(&self) -> f32 {
        if self.muted.load(Ordering::Relaxed) {
            0.0
        } else {
            f32::from_bits(self.volume.load(Ordering::Relaxed))
        }
    }
}

/// A running playback. Dropping it stops playback (the decode thread and device tear
/// down); prefer an explicit [`stop`](AudioPlayer::stop) so `Ended` vs stop stays clear.
pub struct AudioPlayer {
    controls: Arc<Controls>,
}

impl AudioPlayer {
    /// Start playing `input` on the default device, advancing `clock`, reporting to
    /// `events`. Returns immediately; the work runs on a dedicated thread.
    pub fn start(
        input: Box<dyn MediaInput>,
        extension_hint: Option<String>,
        clock: Arc<FrameClock>,
        events: UnboundedSender<EngineEvent>,
    ) -> AudioPlayer {
        let controls = Arc::new(Controls::new());
        let thread_controls = Arc::clone(&controls);
        std::thread::Builder::new()
            .name("canon-audio".into())
            .spawn(move || {
                if let Err(e) = run(
                    input,
                    extension_hint.as_deref(),
                    &clock,
                    &thread_controls,
                    &events,
                ) {
                    // Don't report a failure we caused by stopping.
                    if !thread_controls.stopped.load(Ordering::Relaxed) {
                        let _ = events.send(EngineEvent::Failed(e.to_string()));
                    }
                }
            })
            .expect("spawn audio thread");
        AudioPlayer { controls }
    }

    pub fn pause(&self) {
        self.controls.paused.store(true, Ordering::Relaxed);
    }
    pub fn resume(&self) {
        self.controls.paused.store(false, Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.controls.stopped.store(true, Ordering::Relaxed);
    }
    pub fn set_volume(&self, volume: f32) {
        self.controls
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn set_muted(&self, muted: bool) {
        self.controls.muted.store(muted, Ordering::Relaxed);
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.controls.stopped.store(true, Ordering::Relaxed);
    }
}

/// A [`MediaInput`] presented to Symphonia as a forward, non-seekable stream.
struct ForwardSource {
    inner: Box<dyn MediaInput>,
}

impl Read for ForwardSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}
impl Seek for ForwardSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        // Only the "where am I" query is honoured; real seeking is unsupported on a
        // forward stream (in-track seek is a future feature, canon-e99d).
        self.inner.seek(pos)
    }
}
impl symphonia::core::io::MediaSource for ForwardSource {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

fn run(
    input: Box<dyn MediaInput>,
    extension_hint: Option<&str>,
    clock: &Arc<FrameClock>,
    controls: &Arc<Controls>,
    events: &UnboundedSender<EngineEvent>,
) -> Result<(), crate::output::PlayError> {
    use crate::output::PlayError;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let mss = MediaSourceStream::new(Box::new(ForwardSource { inner: input }), Default::default());
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

    // Decode the first packet to learn the concrete spec before opening the device.
    let mut scratch: Vec<f32> = Vec::new();
    let (sample_rate, channels) = loop {
        if controls.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
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

    let config = crate::output::pick_output_config(sample_rate, channels)?;
    // The clock is reset by the player when it processes the Loaded event below (it owns
    // the clock lifecycle); the callback here only ever *advances* it.
    let channels_usize = channels as usize;
    let capacity = (sample_rate as usize * channels_usize * 500) / 1000;
    let ring_capacity = capacity.max(channels_usize);
    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(ring_capacity);

    let host = cpal::default_host();
    let device = host.default_output_device().ok_or(PlayError::NoDevice)?;
    let cb_clock = Arc::clone(clock);
    let cb_controls = Arc::clone(controls);
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // Paused (or stopped): emit silence and don't advance the clock, so
                // position holds rather than runs on through nothing.
                if cb_controls.paused.load(Ordering::Relaxed)
                    || cb_controls.stopped.load(Ordering::Relaxed)
                {
                    data.iter_mut().for_each(|s| *s = 0.0);
                    return;
                }
                let gain = cb_controls.gain();
                let mut filled = 0usize;
                for slot in data.iter_mut() {
                    match consumer.pop() {
                        Ok(sample) => {
                            *slot = sample * gain;
                            filled += 1;
                        }
                        Err(_) => *slot = 0.0,
                    }
                }
                cb_clock.advance((filled / channels_usize) as u64);
            },
            |err| eprintln!("[audio] output stream error: {err}"),
            None,
        )
        .map_err(|e| PlayError::Device(format!("build stream: {e}")))?;
    stream
        .play()
        .map_err(|e| PlayError::Device(format!("play: {e}")))?;

    // Playback has begun; the player keeps the duration it learned from the track meta.
    let _ = events.send(EngineEvent::Loaded {
        sample_rate,
        duration_ms: None,
    });

    if !push_all(&mut producer, &scratch, controls) {
        return Ok(()); // stopped during pre-roll
    }

    loop {
        if controls.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
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
                if !push_all(&mut producer, &scratch, controls) {
                    return Ok(());
                }
            }
            Err(symphonia::core::errors::Error::DecodeError(_))
            | Err(symphonia::core::errors::Error::IoError(_)) => continue,
            Err(e) => return Err(PlayError::Decode(format!("decode: {e}"))),
        }
    }

    // Natural end: wait for the ring to drain (respecting stop), let the device tail
    // play, then report Ended.
    while producer.slots() < ring_capacity {
        if controls.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(250));
    if !controls.stopped.load(Ordering::Relaxed) {
        let _ = events.send(EngineEvent::Ended);
    }
    Ok(())
}

/// Push all samples into the ring, parking on a full ring but bailing out promptly if
/// stopped. Returns `false` if stopped mid-push.
fn push_all(producer: &mut rtrb::Producer<f32>, samples: &[f32], controls: &Arc<Controls>) -> bool {
    for &sample in samples {
        loop {
            if controls.stopped.load(Ordering::Relaxed) {
                return false;
            }
            match producer.push(sample) {
                Ok(()) => break,
                Err(rtrb::PushError::Full(_)) => std::thread::sleep(Duration::from_millis(2)),
            }
        }
    }
    true
}
