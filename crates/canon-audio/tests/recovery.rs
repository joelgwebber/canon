//! Device-recovery integration test (yak canon-940d).
//!
//! A real unplug / sleep can't be triggered from a harness, so this drives the *same*
//! reopen path via `AudioPlayer::request_reopen` (which reacquires the current default
//! device) and asserts playback survives it: a `DeviceChanged` event is emitted and the
//! frame clock keeps advancing afterwards.
//!
//! Ignored by default — needs a real output device and an audio file. Run with:
//!   CANON_TEST_AUDIO=/path/to/~15s.flac cargo test -p canon-audio --test recovery -- --ignored

use std::sync::Arc;
use std::time::Duration;

use canon_audio::AudioPlayer;
use canon_core::{EngineEvent, FrameClock};

#[tokio::test]
#[ignore = "needs an audio device + CANON_TEST_AUDIO set to a ~15s audio file"]
async fn forced_reopen_keeps_playing() {
    let path =
        std::env::var("CANON_TEST_AUDIO").expect("set CANON_TEST_AUDIO to a ~15s audio file path");
    let file = std::fs::File::open(&path).expect("open test audio");

    let clock = Arc::new(FrameClock::new());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<EngineEvent>();
    let player = AudioPlayer::start(
        Box::new(file),
        Some("flac".into()),
        Arc::clone(&clock),
        tx,
        0,
    );

    // Wait for playback to actually begin.
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(EngineEvent::Loaded { .. })) => break,
            Ok(Some(EngineEvent::Failed(e))) => panic!("failed before playback: {e}"),
            Ok(Some(_)) => {}
            Ok(None) => panic!("engine ended before playback"),
            Err(_) => panic!("timed out waiting for Loaded"),
        }
    }

    tokio::time::sleep(Duration::from_millis(1500)).await;
    let before = clock.frames();
    assert!(before > 0, "clock did not advance before reopen");

    // Force the recovery path (reacquire the current device).
    player.request_reopen();

    // Expect a DeviceChanged event within a few seconds.
    let mut saw_device_changed = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(EngineEvent::DeviceChanged)) => {
                saw_device_changed = true;
                break;
            }
            Ok(Some(EngineEvent::Failed(e))) => panic!("failed during reopen: {e}"),
            Ok(Some(_)) | Err(_) => {}
            Ok(None) => break,
        }
    }
    assert!(saw_device_changed, "no DeviceChanged after request_reopen");

    // And the clock keeps advancing after the reopen.
    let mid = clock.frames();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let after = clock.frames();
    assert!(
        after > mid,
        "clock did not advance after reopen ({mid} -> {after})"
    );

    player.stop();
}
