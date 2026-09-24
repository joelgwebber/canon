//! A controllable, streaming, self-healing audio player (yaks canon-940d + canon-a7d6).
//!
//! [`AudioPlayer::start`] kicks off decode → ring → cpal on a background thread and
//! returns immediately; pause/resume/stop and volume take effect live. Progress is
//! reported as [`EngineEvent`]s so the player state actor stays authoritative.
//!
//! ## Robustness (canon-940d)
//! Output runs as a sequence of *device sessions*. If the device fails — unplugged,
//! sleep/wake, a CoreAudio error — cpal's error callback flips a flag; the engine tears
//! the stream down, reopens the device (preferring the same one by name, else the
//! current default), and continues, emitting [`EngineEvent::DeviceChanged`] so position
//! stays continuous (the tide-2f85 contract, honoured in types). The decoder keeps its
//! place across a reopen, so playback resumes rather than restarts.
//!
//! When the device can't serve the source sample rate, the engine resamples
//! ([`crate::resample`]) instead of erroring, so a 44.1 kHz track still plays on a
//! 48 kHz-only device.
//!
//! ## Gapless hand-off (canon-1838)
//!
//! A playback can be given its successor while it plays ([`AudioPlayer::prepare_next`]). At the
//! end of the current track the feed loop carries straight on into it — same device session, same
//! ring, no drain and no reopen — provided it has the same format (a format change is a break:
//! the track ends and the next one starts as usual). The engine then watches for the listener to
//! actually reach the join: the last samples of the old track are still in the ring when the new
//! one starts decoding. When the clock passes the boundary, the engine moves the clock onto the
//! new track and reports [`EngineEvent::Advanced`].
//!
//! Testability note: a real unplug / sleep can't be triggered from a test harness, so
//! [`AudioPlayer::request_reopen`] forces the same reopen path (reacquiring the current
//! device) to exercise the recovery mechanism end to end.

use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use canon_core::{EngineEvent, FrameClock, MediaInput, PcmSink, PositionDrive};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::mpsc::UnboundedSender;

use crate::error::PlayError;
use crate::resample::LinearResampler;

/// Ring depth in milliseconds of audio.
const RING_MILLIS: u32 = 500;
/// Let the device play out its buffer this long after the ring drains at end of track.
const DRAIN_TAIL: Duration = Duration::from_millis(250);
/// How long to keep retrying to (re)open a device before giving up.
const REOPEN_ATTEMPTS: u32 = 40;
/// Pause between failed (re)open attempts (~40 × 250ms ≈ 10s of grace for sleep/wake).
const REOPEN_BACKOFF: Duration = Duration::from_millis(250);
/// How far ahead of realtime the network output path is allowed to run. A network renderer
/// wants its buffer filled promptly at start, but the producer must not race arbitrarily far
/// ahead of the consumer (that just overruns the stream server's ring); this bounds the lead.
/// It is also why frames fed are not playback position on this path — see [`PositionDrive`].
const NETWORK_LEAD: Duration = Duration::from_secs(2);

/// Where a playback sends its audio.
///
/// The output is chosen per playback at [`AudioPlayer::start`]. Switching sinks is a restart on
/// the new output at the current position (v1 has a single active output), so the engine never
/// re-routes a live stream mid-flight.
pub enum Output {
    /// The local default device (cpal), with device-loss / sleep-wake recovery.
    Local,
    /// A network renderer's PCM sink — e.g. the FLAC encoder tap feeding a Cast stream. The
    /// engine feeds it decoded PCM paced to realtime and advances the clock itself. `joins`:
    /// whether following tracks may be joined on to this stream (flow mode).
    Network { sink: Box<dyn PcmSink>, joins: bool },
}

/// Live, lock-free control shared with the decode thread and the realtime callback.
struct Controls {
    paused: AtomicBool,
    stopped: AtomicBool,
    /// Set by the cpal error callback (or [`AudioPlayer::request_reopen`]) to trigger a
    /// device reopen.
    device_failed: AtomicBool,
    volume: AtomicU32,
    muted: AtomicBool,
}

impl Controls {
    fn new() -> Self {
        Self {
            paused: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            device_failed: AtomicBool::new(false),
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

/// The next track, decoded far enough to know its format, waiting to be joined on.
struct Prepared {
    decode: Decode,
    /// The playback generation it becomes when the listener reaches it.
    to: u64,
}

/// Where a successor waits for the feed loop. Filled from a helper thread, taken at end of track.
type NextSlot = Arc<Mutex<Option<Prepared>>>;

/// A running playback. Dropping it stops playback; prefer explicit [`stop`](Self::stop).
pub struct AudioPlayer {
    controls: Arc<Controls>,
    next: NextSlot,
    /// The last successor [`cancel_next`](Self::cancel_next) gave up on, so one still being
    /// opened is dropped instead of filling the slot afterwards.
    cancelled: Arc<AtomicU64>,
}

impl AudioPlayer {
    pub fn start(
        input: Box<dyn MediaInput>,
        extension_hint: Option<String>,
        clock: Arc<FrameClock>,
        events: UnboundedSender<EngineEvent>,
        start_ms: u64,
        output: Output,
    ) -> AudioPlayer {
        let controls = Arc::new(Controls::new());
        let next: NextSlot = Arc::new(Mutex::new(None));
        let thread_controls = Arc::clone(&controls);
        let thread_next = Arc::clone(&next);
        std::thread::Builder::new()
            .name("canon-audio".into())
            .spawn(move || {
                let hint = extension_hint.as_deref();
                let result = match output {
                    Output::Local => run(
                        input,
                        hint,
                        &clock,
                        &thread_controls,
                        &events,
                        start_ms,
                        &thread_next,
                    ),
                    Output::Network { sink, joins } => run_network(
                        input,
                        hint,
                        &clock,
                        &thread_controls,
                        &events,
                        start_ms,
                        sink,
                        joins,
                        &thread_next,
                    ),
                };
                if let Err(e) = result
                    && !thread_controls.stopped.load(Ordering::Relaxed)
                {
                    let _ = events.send(EngineEvent::Failed(e.to_string()));
                }
            })
            .expect("spawn audio thread");
        AudioPlayer {
            controls,
            next,
            cancelled: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Give this playback its successor, to be joined on without a break at the end of the
    /// current track. It becomes playback `to` when the listener reaches it.
    ///
    /// The decoder is opened here, on its own thread: probing a stream means fetching its first
    /// bytes, and that must never happen on the feed path, whose ring holds half a second. If it
    /// is not ready by the end of the track, or turns out to be a different format, the track just
    /// ends and the successor is started as usual. On a network output it is joined on to the same
    /// stream (flow mode): the renderer never sees the boundary.
    pub fn prepare_next(
        &self,
        input: Box<dyn MediaInput>,
        extension_hint: Option<String>,
        to: u64,
    ) {
        let slot = Arc::clone(&self.next);
        let cancelled = Arc::clone(&self.cancelled);
        let spawned = std::thread::Builder::new()
            .name("canon-audio-next".into())
            .spawn(
                move || match Decode::open(input, extension_hint.as_deref()) {
                    Ok(decode) => {
                        let mut slot = slot.lock().expect("next slot poisoned");
                        if cancelled.load(Ordering::Acquire) != to {
                            *slot = Some(Prepared { decode, to });
                        }
                    }
                    Err(e) => tracing::warn!("next track not prepared for a gapless join: {e}"),
                },
            );
        if let Err(e) = spawned {
            tracing::warn!("next track not prepared for a gapless join: {e}");
        }
    }

    /// Give up on successor `to`: the queue no longer has it next. Too late if it has already
    /// been joined on; the player handles that when it hears the join.
    pub fn cancel_next(&self, to: u64) {
        self.cancelled.store(to, Ordering::Release);
        let mut slot = self.next.lock().expect("next slot poisoned");
        if slot.as_ref().is_some_and(|prepared| prepared.to == to) {
            *slot = None;
        }
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
    /// Force a device reopen. Diagnostic hook to exercise recovery without a real device
    /// change (see the module note).
    pub fn request_reopen(&self) {
        self.controls.device_failed.store(true, Ordering::Relaxed);
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

/// The decode half: owns the demuxer/decoder and yields interleaved source-rate chunks.
struct Decode {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
    track_id: u32,
    first: Option<Vec<f32>>,
    source_rate: u32,
    channels: u16,
}

impl Decode {
    fn open(input: Box<dyn MediaInput>, extension_hint: Option<&str>) -> Result<Decode, PlayError> {
        use symphonia::core::codecs::audio::AudioDecoderOptions;
        use symphonia::core::formats::probe::Hint;
        use symphonia::core::formats::{FormatOptions, TrackType};
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;

        let mss =
            MediaSourceStream::new(Box::new(ForwardSource { inner: input }), Default::default());
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
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| PlayError::Decode("no codec params".into()))?
            .clone();
        let mut decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| PlayError::Decode(format!("no decoder: {e}")))?;

        let mut first = Vec::new();
        let (source_rate, channels) = loop {
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
                    decoded.copy_to_vec_interleaved(&mut first);
                    break (rate, ch);
                }
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(e) => return Err(PlayError::Decode(format!("decode: {e}"))),
            }
        };
        if source_rate == 0 || channels == 0 {
            return Err(PlayError::Decode("stream reported no rate/channels".into()));
        }
        Ok(Decode {
            format,
            decoder,
            track_id,
            first: Some(first),
            source_rate,
            channels,
        })
    }

    /// The next interleaved source-rate chunk, or `None` at end of stream.
    fn next(&mut self) -> Result<Option<Vec<f32>>, PlayError> {
        if let Some(first) = self.first.take() {
            return Ok(Some(first));
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                Err(symphonia::core::errors::Error::ResetRequired) => return Ok(None),
                Err(e) => return Err(PlayError::Decode(format!("demux: {e}"))),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let mut buf = Vec::new();
                    decoded.copy_to_vec_interleaved(&mut buf);
                    return Ok(Some(buf));
                }
                Err(symphonia::core::errors::Error::DecodeError(_))
                | Err(symphonia::core::errors::Error::IoError(_)) => continue,
                Err(e) => return Err(PlayError::Decode(format!("decode: {e}"))),
            }
        }
    }
}

/// A join the listener has not reached yet: where the old track ends on the clock, and the
/// playback that follows it.
#[derive(Clone, Copy)]
struct Crossing {
    boundary: u64,
    to: u64,
}

/// Once the listener has reached the join, move the clock onto the new track and say so.
fn cross_if_reached(
    crossing: &mut Option<Crossing>,
    clock: &FrameClock,
    events: &UnboundedSender<EngineEvent>,
) {
    if let Some(Crossing { boundary, to }) = *crossing
        && clock.frames() >= boundary
    {
        clock.rebase(boundary);
        let _ = events.send(EngineEvent::Advanced { to });
        *crossing = None;
    }
}

/// The prepared successor, if it is ready and can be joined on: the same source format, so the
/// same device session and resampler carry straight on.
fn take_joinable(next: &NextSlot, source_rate: u32, channels: u16) -> Option<Prepared> {
    let prepared = next.lock().expect("next slot poisoned").take()?;
    if prepared.decode.source_rate == source_rate && prepared.decode.channels == channels {
        Some(prepared)
    } else {
        tracing::debug!(
            "next track is {} Hz × {}, not {source_rate} Hz × {channels}: a break, not a join",
            prepared.decode.source_rate,
            prepared.decode.channels
        );
        None
    }
}

fn run(
    input: Box<dyn MediaInput>,
    extension_hint: Option<&str>,
    clock: &Arc<FrameClock>,
    controls: &Arc<Controls>,
    events: &UnboundedSender<EngineEvent>,
    start_ms: u64,
    next: &NextSlot,
) -> Result<(), PlayError> {
    let mut decode = Decode::open(input, extension_hint)?;
    let source_rate = decode.source_rate;
    let channels = decode.channels;

    let mut preferred_name: Option<String> = None;
    let mut first_session = true;
    // Frames of the current track fed to the device, on the clock's scale: where on the clock the
    // track will have ended once everything fed so far has been heard.
    let mut fed: u64 = 0;
    let mut crossing: Option<Crossing> = None;

    // Device-session loop: each iteration opens a device and plays until it ends, is
    // stopped, or the device fails (then we reopen).
    loop {
        if controls.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
        controls.device_failed.store(false, Ordering::Relaxed);

        let session = open_session(
            preferred_name.as_deref(),
            source_rate,
            channels,
            clock,
            controls,
        )?;
        preferred_name = Some(session.device_name.clone());
        let mut producer = session.producer;
        let device_rate = session.device_rate;
        let _stream = session.stream; // kept alive for the session
        let mut resampler = (device_rate != source_rate)
            .then(|| LinearResampler::new(source_rate, device_rate, channels));

        if first_session {
            // The player resets the clock to this rate and seeks it to start_ms; position
            // = frames/device_rate is real elapsed time whether or not we resample.
            let _ = events.send(EngineEvent::Loaded {
                sample_rate: device_rate,
                duration_ms: None,
                start_ms,
                drive: PositionDrive::Frames,
                joins: true,
            });
            // The same arithmetic `FrameClock::seek` uses, so `fed` and the clock agree.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                fed =
                    (Duration::from_millis(start_ms).as_secs_f64() * f64::from(device_rate)) as u64;
            }
            first_session = false;
        } else {
            let _ = events.send(EngineEvent::DeviceChanged);
        }

        let mut ended = false;
        'feed: loop {
            if controls.stopped.load(Ordering::Relaxed) {
                return Ok(());
            }
            if controls.device_failed.load(Ordering::Relaxed) {
                break 'feed; // reopen
            }
            cross_if_reached(&mut crossing, clock, events);
            let source = match decode.next()? {
                Some(chunk) => chunk,
                // End of this track. Carry straight on into its successor if there is one we can
                // join — one join in flight at a time, so a track shorter than the ring ends
                // normally rather than stacking joins the clock hasn't reached.
                None => match crossing
                    .is_none()
                    .then(|| take_joinable(next, source_rate, channels))
                    .flatten()
                {
                    Some(prepared) => {
                        crossing = Some(Crossing {
                            boundary: fed,
                            to: prepared.to,
                        });
                        decode = prepared.decode;
                        fed = 0; // counted from the join, which the clock is rebased onto
                        continue 'feed;
                    }
                    None => {
                        ended = true;
                        break 'feed;
                    }
                },
            };
            let samples = match resampler.as_mut() {
                Some(r) => r.process(&source),
                None => source,
            };
            if !push_all(&mut producer, &samples, controls) {
                if controls.stopped.load(Ordering::Relaxed) {
                    return Ok(());
                }
                break 'feed; // device failed mid-push -> reopen
            }
            fed += (samples.len() / channels as usize) as u64;
        }

        if ended {
            // Wait for the ring to drain (respecting stop / a late device failure).
            let ring_capacity = producer.buffer().capacity();
            while producer.slots() < ring_capacity {
                if controls.stopped.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if controls.device_failed.load(Ordering::Relaxed) {
                    break;
                }
                cross_if_reached(&mut crossing, clock, events);
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(DRAIN_TAIL);
            // A join the ring drained past without the check catching it (a very short final
            // track): the listener has heard it, so report it before the end.
            if let Some(Crossing { boundary, to }) = crossing.take() {
                clock.rebase(boundary);
                let _ = events.send(EngineEvent::Advanced { to });
            }
            if !controls.stopped.load(Ordering::Relaxed)
                && !controls.device_failed.load(Ordering::Relaxed)
            {
                let _ = events.send(EngineEvent::Ended);
                return Ok(());
            }
            // A device failure during drain: fall through to reopen and finish there.
        }
        // Drop `_stream` at end of scope, then loop to reopen.
    }
}

/// Network output: decode → submit PCM to a [`PcmSink`] (the FLAC encoder tap), paced to
/// wall-clock realtime and kept ~[`NETWORK_LEAD`] ahead so the renderer's buffer stays fed
/// without the producer racing far past the consumer.
///
/// The [`FrameClock`] is advanced here by frames *fed to the encoder*, which is a genuine
/// measure of how far ahead of realtime this loop is running — but it is not where the
/// listener is, and the player does not read it as position for this stream. The renderer
/// reports that, and the player reconciles to it (`PositionDrive::Renderer`).
///
/// Volume/mute are *not* applied here: a network renderer controls its own volume (see the sink's
/// `set_volume`), so scaling the PCM would double it. End-of-stream does **not** emit
/// [`EngineEvent::Ended`] — the renderer is still playing out its buffer after we stop feeding, so
/// completion is reported by the sink's own status feedback (Cast MEDIA_STATUS), not by frames
/// fed. Returning drops `sink`, whose `Drop` flushes its trailing frame.
///
/// In flow mode a prepared successor of the same format is joined on at end of track, into the
/// same sink — one stream, no boundary the renderer can see. The join is reported with its time on
/// the stream ([`EngineEvent::Joined`]); when the listener actually reaches it is the renderer's to
/// say, through its position reports.
#[allow(clippy::too_many_arguments)]
fn run_network(
    input: Box<dyn MediaInput>,
    extension_hint: Option<&str>,
    clock: &Arc<FrameClock>,
    controls: &Arc<Controls>,
    events: &UnboundedSender<EngineEvent>,
    start_ms: u64,
    mut sink: Box<dyn PcmSink>,
    joins: bool,
    next: &NextSlot,
) -> Result<(), PlayError> {
    let mut decode = Decode::open(input, extension_hint)?;
    let source_rate = decode.source_rate;
    let channels = decode.channels;

    // The player actor resets the clock to this rate and seeks it to start_ms. Position on
    // this path comes from the renderer, not from us: see `PositionDrive::Renderer`.
    let _ = events.send(EngineEvent::Loaded {
        sample_rate: source_rate,
        duration_ms: None,
        start_ms,
        drive: PositionDrive::Renderer,
        joins,
    });

    let mut play_start = Instant::now();
    let mut frames_fed: u64 = 0;

    loop {
        if controls.stopped.load(Ordering::Relaxed) {
            return Ok(());
        }
        if controls.paused.load(Ordering::Relaxed) {
            let paused_at = Instant::now();
            while controls.paused.load(Ordering::Relaxed)
                && !controls.stopped.load(Ordering::Relaxed)
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            // Paused time must not count against pacing, or we'd sprint to catch up on resume.
            play_start += paused_at.elapsed();
            continue;
        }

        let chunk = match decode.next()? {
            Some(chunk) => chunk,
            None => match joins
                .then(|| take_joinable(next, source_rate, channels))
                .flatten()
            {
                // Flow: carry straight on into the successor, on the same stream.
                Some(prepared) => {
                    let at = Duration::from_secs_f64(frames_fed as f64 / f64::from(source_rate));
                    let _ = events.send(EngineEvent::Joined {
                        to: prepared.to,
                        at,
                    });
                    decode = prepared.decode;
                    continue;
                }
                // Exhausted; Ended arrives via the sink's status feedback.
                None => {
                    tracing::debug!(
                        fed_secs = frames_fed as f64 / f64::from(source_rate),
                        "network feed: source exhausted"
                    );
                    return Ok(());
                }
            },
        };
        let frames = (chunk.len() / channels as usize) as u64;
        sink.submit(&chunk, source_rate, channels);
        // Frames fed, not frames heard — the renderer is ~NETWORK_LEAD plus its own buffer
        // behind this point.
        clock.advance(frames);
        frames_fed += frames;

        // Stay ~NETWORK_LEAD ahead of realtime; sleep off any excess.
        let target =
            play_start + Duration::from_secs_f64(frames_fed as f64 / f64::from(source_rate));
        if let Some(ahead) = target.checked_duration_since(Instant::now())
            && ahead > NETWORK_LEAD
        {
            pace_sleep(ahead - NETWORK_LEAD, controls);
        }
    }
}

/// Sleep `dur`, waking promptly on stop/pause so control latency stays low even mid-pace.
fn pace_sleep(dur: Duration, controls: &Arc<Controls>) {
    let end = Instant::now() + dur;
    loop {
        if controls.stopped.load(Ordering::Relaxed) || controls.paused.load(Ordering::Relaxed) {
            return;
        }
        let now = Instant::now();
        if now >= end {
            return;
        }
        std::thread::sleep((end - now).min(Duration::from_millis(20)));
    }
}

/// A live output session: the device, its stream, the ring producer, and the rate.
struct Session {
    stream: cpal::Stream,
    producer: rtrb::Producer<f32>,
    device_rate: u32,
    device_name: String,
}

/// Open (or reopen) a device session, retrying with backoff so a brief disappearance
/// (sleep/wake) heals rather than fails.
fn open_session(
    preferred_name: Option<&str>,
    source_rate: u32,
    channels: u16,
    clock: &Arc<FrameClock>,
    controls: &Arc<Controls>,
) -> Result<Session, PlayError> {
    let mut last_err = PlayError::NoDevice;
    for _ in 0..REOPEN_ATTEMPTS {
        if controls.stopped.load(Ordering::Relaxed) {
            return Err(PlayError::Device("stopped while opening device".into()));
        }
        match try_open_session(preferred_name, source_rate, channels, clock, controls) {
            Ok(session) => return Ok(session),
            Err(e) => {
                last_err = e;
                std::thread::sleep(REOPEN_BACKOFF);
            }
        }
    }
    Err(last_err)
}

fn try_open_session(
    preferred_name: Option<&str>,
    source_rate: u32,
    channels: u16,
    clock: &Arc<FrameClock>,
    controls: &Arc<Controls>,
) -> Result<Session, PlayError> {
    let (device, device_name) = resolve_device(preferred_name)?;
    let (config, device_rate) = choose_config(&device, source_rate, channels)?;

    let ring_capacity = ((device_rate as usize * channels as usize * RING_MILLIS as usize) / 1000)
        .max(channels as usize);
    let (producer, mut consumer) = rtrb::RingBuffer::<f32>::new(ring_capacity);

    let cb_clock = Arc::clone(clock);
    let channels_usize = channels as usize;
    let cb_controls = Arc::clone(controls);
    let err_controls = Arc::clone(controls);
    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
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
            move |err| {
                eprintln!("[audio] output stream error: {err}");
                err_controls.device_failed.store(true, Ordering::Relaxed);
            },
            None,
        )
        .map_err(|e| PlayError::Device(format!("build stream: {e}")))?;
    stream
        .play()
        .map_err(|e| PlayError::Device(format!("play: {e}")))?;

    Ok(Session {
        stream,
        producer,
        device_rate,
        device_name,
    })
}

/// Resolve the output device, preferring the same one by name (stable identity across a
/// disconnect/reconnect) and falling back to the current default.
fn resolve_device(preferred_name: Option<&str>) -> Result<(cpal::Device, String), PlayError> {
    let host = cpal::default_host();
    if let Some(name) = preferred_name
        && let Ok(devices) = host.output_devices()
    {
        for device in devices {
            if device.name().ok().as_deref() == Some(name) {
                return Ok((device, name.to_string()));
            }
        }
    }
    let device = host.default_output_device().ok_or(PlayError::NoDevice)?;
    let name = device.name().unwrap_or_else(|_| "default".to_string());
    Ok((device, name))
}

/// Choose an `f32` output config: prefer the source rate (bit-perfect), else fall back to
/// the device's default rate (the engine resamples to it).
fn choose_config(
    device: &cpal::Device,
    source_rate: u32,
    channels: u16,
) -> Result<(cpal::StreamConfig, u32), PlayError> {
    let target = cpal::SampleRate(source_rate);
    let ranges: Vec<_> = device
        .supported_output_configs()
        .map_err(|e| PlayError::Device(format!("query configs: {e}")))?
        .filter(|r| r.channels() == channels && r.sample_format() == cpal::SampleFormat::F32)
        .collect();

    // Exact source rate if the device supports it.
    for range in &ranges {
        if range.min_sample_rate() <= target && target <= range.max_sample_rate() {
            return Ok(((*range).with_sample_rate(target).config(), source_rate));
        }
    }
    // Otherwise the device's default rate (the engine resamples to it).
    if let Ok(default) = device.default_output_config()
        && default.sample_format() == cpal::SampleFormat::F32
        && default.channels() == channels
    {
        let rate = default.sample_rate().0;
        return Ok((default.config(), rate));
    }
    if let Some(range) = ranges.first() {
        let rate = range.max_sample_rate();
        return Ok(((*range).with_sample_rate(rate).config(), rate.0));
    }
    Err(PlayError::UnsupportedFormat(format!(
        "device has no f32 output config for {channels} channels"
    )))
}

/// Push all samples into the ring, parking on a full ring but bailing promptly on stop or
/// device failure. Returns `false` if interrupted.
fn push_all(producer: &mut rtrb::Producer<f32>, samples: &[f32], controls: &Arc<Controls>) -> bool {
    for &sample in samples {
        loop {
            if controls.stopped.load(Ordering::Relaxed)
                || controls.device_failed.load(Ordering::Relaxed)
            {
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
