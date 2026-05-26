//! IR `.wav` corrective operations used by `qa_fix` (issue #12).
//!
//! All operations are pure on sample buffers, TDD-covered with both
//! a passing and a defect-correcting case. The `qa_fix` binary wraps
//! these to load → fix → write back IR files so the catalogue can
//! pass `qa_audit` without hand-editing each `.wav`.

use crate::ir::convolve;
use crate::loudness::peak_dbfs;

/// Subtracts the buffer's mean from every sample. Eliminates DC offset.
pub fn dc_remove(samples: &[f32]) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let mean = samples.iter().map(|s| *s as f64).sum::<f64>() / samples.len() as f64;
    samples.iter().map(|s| (*s as f64 - mean) as f32).collect()
}

/// Windowed-sinc resampler (Hann window over a 32-tap kernel per output
/// sample). Replaces linear interpolation for IR rate conversion; the
/// kernel is wide enough to keep imaging well below the QA HF-aliasing
/// threshold while staying cheap enough for offline use across the
/// catalogue.
pub fn sinc_resample(samples: &[f32], src_sr: u32, dst_sr: u32) -> Vec<f32> {
    if src_sr == dst_sr || samples.is_empty() {
        return samples.to_vec();
    }
    const HALF_TAPS: i32 = 16;
    let ratio = dst_sr as f64 / src_sr as f64;
    let cutoff = if ratio < 1.0 { ratio } else { 1.0 };
    let out_len = (samples.len() as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = (i as f64) / ratio;
        let centre = src_pos.floor() as i32;
        let frac = src_pos - centre as f64;
        let mut acc = 0.0_f64;
        for k in (-HALF_TAPS + 1)..=HALF_TAPS {
            let idx = centre + k;
            if idx < 0 || (idx as usize) >= samples.len() {
                continue;
            }
            let x = (k as f64 - frac) * cutoff;
            let w = hann_window(k as f64 - frac, HALF_TAPS as f64);
            let s = sinc(x);
            acc += samples[idx as usize] as f64 * s * w * cutoff;
        }
        out.push(acc as f32);
    }
    out
}

#[inline]
fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

#[inline]
fn hann_window(x: f64, half_width: f64) -> f64 {
    if x.abs() >= half_width {
        0.0
    } else {
        0.5 * (1.0 + (std::f64::consts::PI * x / half_width).cos())
    }
}

/// Full `qa_fix` per-channel pipeline as a pure function: DC-remove
/// then sinc-resample to `dst_sr`. Deliberately level-preserving — the
/// previous pipeline scaled each capture so its convolution with the
/// synthetic DI peaked at -1 dBFS, which made every IR ship hot and
/// hid the natural insertion loss the boost-only audit (#4) needs to
/// see. Issue #21.
pub fn fix_capture(samples: &[f32], src_sr: u32, dst_sr: u32) -> Vec<f32> {
    let centred = dc_remove(samples);
    if src_sr == dst_sr {
        centred
    } else {
        sinc_resample(&centred, src_sr, dst_sr)
    }
}

/// Ceiling-only convolution scale: if `convolve(probe, ir).peak >
/// ceiling_dbfs`, scale `ir` so the peak lands exactly at the ceiling;
/// otherwise return `ir` unchanged. Unlike a target peak-norm, quiet
/// IRs keep their natural insertion loss (the boost-only audit needs
/// that signal); only the intrinsically-hot captures are tamed so the
/// CLIP threshold in `qa_audit` is not violated. Issue #21.
pub fn scale_to_convolution_ceiling(
    probe: &[f32],
    ir: &[f32],
    ceiling_dbfs: f32,
) -> Vec<f32> {
    let wet = convolve(probe, ir);
    let measured = peak_dbfs(&wet);
    if !measured.is_finite() || measured <= ceiling_dbfs {
        return ir.to_vec();
    }
    let scale = 10f32.powf((ceiling_dbfs - measured) / 20.0);
    ir.iter().map(|s| s * scale).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

    fn sr() -> u32 {
        DI_SAMPLE_RATE as u32
    }

    // --- dc_remove --- //

    #[test]
    fn dc_remove_passes_through_centred_signal() {
        let di = default_guitar_di();
        let cleaned = dc_remove(&di);
        let mean: f32 = cleaned.iter().sum::<f32>() / cleaned.len() as f32;
        assert!(mean.abs() < 1e-6);
    }

    #[test]
    fn dc_remove_corrects_dc_offset() {
        let buf: Vec<f32> = (0..1000).map(|_| 0.5_f32).collect();
        let cleaned = dc_remove(&buf);
        let mean: f32 = cleaned.iter().sum::<f32>() / cleaned.len() as f32;
        assert!(mean.abs() < 1e-6);
    }

    // --- sinc_resample --- //

    #[test]
    fn sinc_resample_noop_when_rates_equal() {
        let x = vec![0.1, 0.2, 0.3];
        assert_eq!(sinc_resample(&x, 48_000, 48_000), x);
    }

    #[test]
    fn sinc_resample_44k_to_48k_length_within_one_sample() {
        let x: Vec<f32> = (0..441).map(|i| (i as f32 * 0.07).sin()).collect();
        let y = sinc_resample(&x, 44_100, 48_000);
        assert!((y.len() as i32 - 480).abs() <= 1, "len was {}", y.len());
    }

    #[test]
    fn sinc_resample_does_not_introduce_high_frequency_garbage() {
        // A pure 1 kHz sine resampled 44.1→48 stays a near-pure 1 kHz:
        // the energy above 18 kHz must remain tiny (sinc imaging
        // suppressed). Linear interp fails this; that is the regression
        // this resampler exists to prevent.
        use crate::qa::check_hf_aliasing;
        let n = 44_100; // 1 s @ 44.1k
        let probe: Vec<f32> = (0..n)
            .map(|i| 0.5 * (std::f32::consts::TAU * 1_000.0 * i as f32 / 44_100.0).sin())
            .collect();
        let out = sinc_resample(&probe, 44_100, 48_000);
        // Use the resampled output AS BOTH probe and out to ratio against
        // itself isn't useful — instead synthesise a 48k reference of the
        // same 1 kHz sine and compare HF energy.
        let n48 = 48_000;
        let reference: Vec<f32> = (0..n48)
            .map(|i| 0.5 * (std::f32::consts::TAU * 1_000.0 * i as f32 / 48_000.0).sin())
            .collect();
        assert!(
            check_hf_aliasing(&reference, &out, 48_000).is_none(),
            "sinc resampler must not produce HF imaging above the QA threshold"
        );
    }

    // --- fix_capture (regression guard, issue #21) --- //

    #[test]
    fn fix_capture_is_identity_for_dc_free_signal_at_same_sample_rate() {
        // DC-free input + matching sample rate: pipeline must be a no-op.
        // This pins down "no implicit scaling / no peak-norm" at same SR.
        let original = vec![-0.5_f32, 0.5, -0.5, 0.5];
        let fixed = fix_capture(&original, 48_000, 48_000);
        assert_eq!(fixed, original);
        let _ = sr(); // silence unused-helper warn in narrow tests
    }

    #[test]
    fn fix_capture_preserves_peak_across_resample() {
        // Defect being guarded: the previous qa_fix scaled every IR so
        // its convolution with the synthetic DI peaked at -1 dBFS,
        // turning natural-loss captures into hot ones. fix_capture must
        // pass the level through unchanged across the resample.
        let original: Vec<f32> = (0..441)
            .map(|i| 0.3 * (std::f32::consts::TAU * 1_000.0 * i as f32 / 44_100.0).sin())
            .collect();
        let in_peak = original.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        let fixed = fix_capture(&original, 44_100, 48_000);
        let out_peak = fixed.iter().fold(0.0_f32, |a, b| a.max(b.abs()));
        assert!(
            (in_peak - out_peak).abs() < 0.05,
            "peak changed: {in_peak:.3} -> {out_peak:.3}",
        );
    }

    // --- scale_to_convolution_ceiling --- //

    #[test]
    fn ceiling_scales_down_when_convolution_exceeds_ceiling() {
        // Hot IR (delta scaled by 8) → convolved peak well above 0 dBFS.
        // Must be brought down to exactly the ceiling.
        let di = default_guitar_di();
        let ir = vec![8.0_f32];
        let out = scale_to_convolution_ceiling(&di, &ir, -1.0);
        let wet = convolve(&di, &out);
        let p = peak_dbfs(&wet);
        assert!((p - (-1.0)).abs() < 0.05, "peak was {p:.3} dBFS");
        let _ = sr();
    }

    #[test]
    fn ceiling_passes_quiet_ir_through_unchanged() {
        // Quiet IR (attenuating delta) → convolution stays well below
        // the ceiling. The ceiling MUST NOT lift quiet IRs (that was
        // the peak-norm bug). Output is the same buffer as input.
        let di = default_guitar_di();
        let ir = vec![0.1_f32];
        let out = scale_to_convolution_ceiling(&di, &ir, -1.0);
        assert_eq!(out, ir, "quiet IR was scaled when it should have passed through");
    }
}
