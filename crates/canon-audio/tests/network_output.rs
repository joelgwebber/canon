//! The engine's network output path (yak canon-5df5).
//!
//! Proves that selecting a network output feeds decoded PCM to a [`PcmSink`] and advances the
//! shared [`FrameClock`], without a real audio device — the same seam the Chromecast sink drives
//! (its FLAC encoder tap is the real `PcmSink`). Uses the committed fragmented-FLAC asset so the
//! decode path is exercised end to end.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use canon_audio::{AudioPlayer, Output};
use canon_core::{FrameClock, MediaInput, PcmSink};

/// A minimal `PcmSink` that just tallies the interleaved samples it is handed, so a test can
/// observe that the engine actually fed the network path.
struct CountingSink {
    samples: Arc<AtomicU64>,
}

impl PcmSink for CountingSink {
    fn submit(&mut self, frames: &[f32], _sample_rate: u32, _channels: u16) {
        self.samples
            .fetch_add(frames.len() as u64, Ordering::Relaxed);
    }
}

fn asset_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("read asset {name}: {e}"))
}

#[test]
fn network_output_feeds_pcm_and_advances_the_clock() {
    let input: Box<dyn MediaInput> = Box::new(Cursor::new(asset_bytes("flac_frag.mp4")));
    let clock = Arc::new(FrameClock::new());
    // Give the clock a rate so a position could be derived; the assertion uses raw frames, which
    // is independent of rate, but this mirrors what the player actor does on `Loaded`.
    clock.reset(44_100);
    let (events_tx, _events_rx) = tokio::sync::mpsc::unbounded_channel();
    let samples = Arc::new(AtomicU64::new(0));
    let sink = Box::new(CountingSink {
        samples: Arc::clone(&samples),
    });

    let player = AudioPlayer::start(
        input,
        Some("mp4".to_string()),
        Arc::clone(&clock),
        events_tx,
        0,
        Output::Network(sink),
    );

    // The lead lets a couple of seconds of audio feed near-instantly; poll briefly for it.
    let mut waited = Duration::ZERO;
    while samples.load(Ordering::Relaxed) == 0 && waited < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
        waited += Duration::from_millis(20);
    }
    player.stop();

    assert!(
        samples.load(Ordering::Relaxed) > 0,
        "network sink should have received decoded PCM"
    );
    assert!(
        clock.frames() > 0,
        "the clock should advance by frames fed on the network path"
    );
}
