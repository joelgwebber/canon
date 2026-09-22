//! Fragmented-MP4 de-risking spike (yak canon-c4c3).
//!
//! The riskiest assumption in the audio plan: Tidal delivers FLAC and AAC inside
//! *fragmented* MP4 (`moof`/`mdat` segments with an `empty_moov`), and Symphonia's
//! `isomp4` support for fragmented MP4 was unverified. These tests answer it
//! empirically against three committed assets:
//!
//! * `aac_plain.m4a`  — non-fragmented AAC. CONTROL. If this fails, Symphonia can't
//!   do MP4 at all, which is a different problem than the fragmented one.
//! * `aac_frag.mp4`   — AAC in fragmented MP4 (`-movflags frag_keyframe+empty_moov+default_base_moof`).
//! * `flac_frag.mp4`  — FLAC in fragmented MP4 (same movflags).
//!
//! Top-level box order (verified at asset-generation time):
//!   fragmented: `ftyp, moov, moof, mdat, mfra`   (moof/mfra = fragmentation)
//!   control:    `ftyp, free, mdat, moov`         (no moof)
//!
//! Each decode is wrapped in `catch_unwind` so that a demuxer which chokes on a
//! `moof` box surfaces as a reported error instead of crashing the whole suite —
//! per the spike brief, "Symphonia panics on the moof box" is itself a valid
//! finding worth capturing cleanly.

use std::io::{Cursor, Read, Seek};
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;

use canon_audio::{Codec, DecodeSummary, MediaInput, decode};

fn asset_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/assets")
        .join(name)
}

fn asset_bytes(name: &str) -> Vec<u8> {
    std::fs::read(asset_path(name)).unwrap_or_else(|e| panic!("read asset {name}: {e}"))
}

/// Run the decode seam over a reader, converting any panic into an `Err(String)`
/// so a hostile container cannot abort the test process.
fn run<R: Read + Seek + Send + Sync + 'static>(
    input: R,
    ext: &str,
) -> Result<DecodeSummary, String> {
    match panic::catch_unwind(AssertUnwindSafe(|| decode(input, Some(ext)))) {
        Ok(Ok(summary)) => Ok(summary),
        Ok(Err(e)) => Err(format!("decode error: {e}")),
        Err(_) => Err("PANIC inside Symphonia decode".to_string()),
    }
}

/// Decode `name` through the production-relevant seam: a `Box<dyn MediaInput>` over
/// the bytes, the exact analogue of what the Tidal source resolves into
/// `ResolvedStream.input` and hands the audio layer.
fn decode_boxed(name: &str, ext: &str) -> Result<DecodeSummary, String> {
    let boxed: Box<dyn MediaInput> = Box::new(Cursor::new(asset_bytes(name)));
    run(boxed, ext)
}

// --- CONTROL: non-fragmented AAC must decode. -------------------------------

#[test]
fn control_plain_aac_decodes() {
    // Path 1: a real File (File is coincidentally Send + Sync).
    let file = std::fs::File::open(asset_path("aac_plain.m4a")).expect("open control asset");
    let from_file = run(file, "m4a").expect("control (File seam) must decode");

    // Path 2: a boxed trait object over in-memory bytes — the MediaInput-style seam.
    let from_boxed =
        decode_boxed("aac_plain.m4a", "m4a").expect("control (boxed seam) must decode");

    eprintln!("[control aac_plain.m4a] file={from_file:?}");
    eprintln!("[control aac_plain.m4a] boxed={from_boxed:?}");

    assert_eq!(from_file.codec, Codec::Aac, "control codec should be AAC");
    assert!(from_file.packets > 0, "control must yield packets");
    assert!(
        from_file.frames_decoded > 0,
        "control must decode audio frames"
    );
    assert_eq!(from_file.decode_errors, 0, "control must decode cleanly");

    // Both seam paths must agree — proves the boxed boundary is equivalent to File.
    assert_eq!(
        from_file.frames_decoded, from_boxed.frames_decoded,
        "File and boxed seams must decode identically"
    );
}

// --- THE RISK: fragmented MP4. ----------------------------------------------

#[test]
fn fragmented_aac_verdict() {
    let result = decode_boxed("aac_frag.mp4", "mp4");
    eprintln!("[VERDICT aac_frag.mp4 (fragmented)] {result:?}");

    let summary = result.expect(
        "fragmented AAC failed to decode — this CONFIRMS the fMP4 risk; \
         see printed error and the spike report",
    );
    assert_eq!(summary.codec, Codec::Aac);
    assert!(
        summary.packets > 0,
        "fragmented AAC probed but produced no packets (moof ignored?)"
    );
    assert!(
        summary.frames_decoded > 0,
        "fragmented AAC produced no decoded frames"
    );
}

#[test]
fn fragmented_flac_verdict() {
    let result = decode_boxed("flac_frag.mp4", "mp4");
    eprintln!("[VERDICT flac_frag.mp4 (fragmented)] {result:?}");

    let summary = result.expect(
        "fragmented FLAC failed to decode — this CONFIRMS the fMP4 risk; \
         see printed error and the spike report",
    );
    assert_eq!(summary.codec, Codec::Flac);
    assert!(
        summary.packets > 0,
        "fragmented FLAC probed but produced no packets (moof ignored?)"
    );
    assert!(
        summary.frames_decoded > 0,
        "fragmented FLAC produced no decoded frames"
    );
}

/// Cross-check: the fragmented and plain AAC assets carry the same 2s 440Hz signal,
/// so a working fMP4 demuxer should decode a comparable number of frames to the
/// control. A gross mismatch (e.g. only the first fragment) would betray partial
/// fMP4 handling even if probing "succeeds".
#[test]
fn fragmented_aac_decodes_full_stream() {
    let plain = match decode_boxed("aac_plain.m4a", "m4a") {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[skip] control did not decode ({e}); covered by control test");
            return;
        }
    };
    let frag = match decode_boxed("aac_frag.mp4", "mp4") {
        Ok(s) => s,
        Err(e) => panic!("fragmented AAC did not decode fully — confirms fMP4 risk: {e}"),
    };

    eprintln!(
        "[full-stream aac] plain={} frames, frag={} frames",
        plain.frames_decoded, frag.frames_decoded
    );

    // Allow one AAC frame (1024 samples) of slack for encoder priming/padding
    // differences between the two muxings of the same source.
    let diff = plain.frames_decoded.abs_diff(frag.frames_decoded);
    assert!(
        diff <= 2048,
        "fragmented AAC decoded {} frames vs control {} — too far apart, suggests only \
         partial fMP4 demux",
        frag.frames_decoded,
        plain.frames_decoded
    );
}
