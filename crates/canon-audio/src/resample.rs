//! A streaming linear resampler (part of yak canon-940d).
//!
//! When the output device can't serve the source sample rate, canon must resample rather
//! than hard-error. This is a simple per-channel linear interpolator that carries state
//! across chunk boundaries, so it can be fed the variable-length buffers a decoder
//! produces. Linear interpolation is modest fidelity; a higher-quality sinc resampler
//! (e.g. `rubato`) is a worthwhile future upgrade, tracked as a follow-up. It only
//! engages when device rate != source rate, so the common bit-perfect path is untouched.

/// Interleaved-`f32` linear resampler from `in_rate` to `out_rate`.
pub struct LinearResampler {
    channels: usize,
    /// Input frames consumed per output frame (`in_rate / out_rate`).
    step: f64,
    /// Fractional position between `prev` and the current input frame, in [0, 1).
    phase: f64,
    /// The previous input frame (per channel), carried across `process` calls.
    prev: Vec<f32>,
    have_prev: bool,
}

impl LinearResampler {
    pub fn new(in_rate: u32, out_rate: u32, channels: u16) -> Self {
        debug_assert!(in_rate > 0 && out_rate > 0 && channels > 0);
        Self {
            channels: channels as usize,
            step: f64::from(in_rate) / f64::from(out_rate),
            phase: 0.0,
            prev: vec![0.0; channels as usize],
            have_prev: false,
        }
    }

    /// Resample one interleaved chunk, returning interleaved output at `out_rate`.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let ch = self.channels;
        if ch == 0 {
            return Vec::new();
        }
        let frames = input.len() / ch;
        let mut out = Vec::new();
        let mut start = 0;

        if !self.have_prev {
            if frames == 0 {
                return out;
            }
            self.prev.copy_from_slice(&input[0..ch]);
            self.have_prev = true;
            start = 1;
        }

        for f in start..frames {
            let cur = &input[f * ch..f * ch + ch];
            // Emit every output sample whose position falls within [prev, cur).
            while self.phase < 1.0 {
                let t = self.phase as f32;
                for (p, c) in self.prev.iter().zip(cur) {
                    out.push(p + (c - p) * t);
                }
                self.phase += self.step;
            }
            self.phase -= 1.0;
            self.prev.copy_from_slice(cur);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_signal_stays_constant() {
        // A constant input must resample to (near-)constant output at any rate.
        let mut r = LinearResampler::new(44_100, 48_000, 2);
        let input = vec![0.5f32; 2 * 4096];
        let out = r.process(&input);
        assert!(!out.is_empty());
        for s in out {
            assert!((s - 0.5).abs() < 1e-6, "sample drifted: {s}");
        }
    }

    #[test]
    fn upsampling_produces_more_frames() {
        let mut r = LinearResampler::new(44_100, 48_000, 2);
        let in_frames = 44_100;
        let out = r.process(&vec![0.25f32; 2 * in_frames]);
        let out_frames = out.len() / 2;
        // ~48000 out for 44100 in, within a small boundary tolerance.
        let expected = 48_000f64;
        assert!(
            (out_frames as f64 - expected).abs() < 100.0,
            "got {out_frames} out frames, expected ~{expected}"
        );
    }

    #[test]
    fn downsampling_produces_fewer_frames() {
        let mut r = LinearResampler::new(96_000, 48_000, 2);
        let in_frames = 96_000;
        let out = r.process(&vec![0.1f32; 2 * in_frames]);
        let out_frames = out.len() / 2;
        assert!(
            (out_frames as f64 - 48_000.0).abs() < 100.0,
            "got {out_frames} out frames, expected ~48000"
        );
    }

    #[test]
    fn carries_state_across_chunks() {
        // Feeding two halves must yield ~the same count as feeding the whole.
        let mut whole = LinearResampler::new(44_100, 48_000, 1);
        let all = whole.process(&vec![0.3f32; 44_100]);

        let mut split = LinearResampler::new(44_100, 48_000, 1);
        let mut a = split.process(&vec![0.3f32; 22_050]);
        let b = split.process(&vec![0.3f32; 22_050]);
        a.extend(b);

        assert!(
            (all.len() as i64 - a.len() as i64).abs() <= 2,
            "whole {} vs split {}",
            all.len(),
            a.len()
        );
    }
}
