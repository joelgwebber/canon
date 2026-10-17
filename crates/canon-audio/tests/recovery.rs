//! Device-recovery integration test (yak canon-940d).
//!
//! A real unplug / sleep can't be triggered from a harness, so this drives the *same*
//! reopen path via `AudioPlayer::request_reopen` (which reacquires the current default
//! device) and asserts playback survives it: a `DeviceChanged` event is emitted and the
//! reported position keeps advancing afterwards, continuous with where it was.
//!
//! Ignored by default — needs a real output device and an audio file. Run with:
//!   CANON_TEST_AUDIO=/path/to/~15s.flac cargo test -p canon-audio --test recovery -- --ignored

use std::time::Duration;

use canon_audio::AudioPlayer;
use canon_core::EngineEvent;
use tokio::sync::mpsc::UnboundedReceiver;

/// The latest position the engine reported over the next `secs`, failing on a failure.
async fn latest_position(rx: &mut UnboundedReceiver<EngineEvent>, secs: f64) -> Option<Duration> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(secs);
    let mut latest = None;
    while let Ok(event) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match event {
            Some(EngineEvent::LocalPosition(at)) => latest = Some(at),
            Some(EngineEvent::Failed(e)) => panic!("failed: {e}"),
            Some(_) => {}
            None => break,
        }
    }
    latest
}

#[tokio::test]
#[ignore = "needs an audio device + CANON_TEST_AUDIO set to a ~15s audio file"]
async fn forced_reopen_keeps_playing() {
    let path =
        std::env::var("CANON_TEST_AUDIO").expect("set CANON_TEST_AUDIO to a ~15s audio file path");
    let file = std::fs::File::open(&path).expect("open test audio");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<EngineEvent>();
    let player = AudioPlayer::start(
        Box::new(file),
        Some("flac".into()),
        tx,
        0,
        None,
        canon_audio::Output::Local,
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

    let before = latest_position(&mut rx, 1.5).await.unwrap_or_default();
    assert!(!before.is_zero(), "position did not advance before reopen");

    // Force the recovery path (reacquire the current device).
    player.request_reopen();

    // Expect a DeviceChanged event within a few seconds.
    let mut saw_device_changed = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
            Ok(Some(EngineEvent::LocalPosition(at))) => {
                assert!(
                    at >= before,
                    "position went back across the reopen: {before:?} -> {at:?}"
                );
            }
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

    // And position keeps advancing after the reopen, from where it was.
    let mid = latest_position(&mut rx, 0.5)
        .await
        .expect("reports after reopen");
    assert!(mid >= before, "position went back: {before:?} -> {mid:?}");
    let after = latest_position(&mut rx, 1.5)
        .await
        .expect("reports after reopen");
    assert!(
        after > mid,
        "position did not advance after reopen ({mid:?} -> {after:?})"
    );

    player.stop();
}
