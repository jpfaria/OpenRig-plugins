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

/// Scales `ir` so `convolve(probe, scaled_ir)` peaks at exactly
/// `target_peak_dbfs`. The scaling factor is the linear ratio between
/// the desired peak and the measured peak.
pub fn peak_normalize_for_convolution(
    probe: &[f32],
    ir: &[f32],
    target_peak_dbfs: f32,
) -> Vec<f32> {
    let wet = convolve(probe, ir);
    let measured = peak_dbfs(&wet);
    if !measured.is_finite() {
        return ir.to_vec();
    }
    let delta_db = target_peak_dbfs - measured;
    let scale = 10f32.powf(delta_db / 20.0);
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

    // --- peak_normalize_for_convolution --- //

    #[test]
    fn peak_normalize_brings_convolution_peak_to_target() {
        let di = default_guitar_di();
        // Hot IR (delta scaled by 8) → convolved peak well above 0 dBFS.
        let ir = vec![8.0_f32];
        let scaled = peak_normalize_for_convolution(&di, &ir, -1.0);
        let wet = convolve(&di, &scaled);
        let p = peak_dbfs(&wet);
        assert!((p - (-1.0)).abs() < 0.05, "peak was {p:.3} dBFS");
    }

    #[test]
    fn peak_normalize_passing_case_already_in_range() {
        let di = default_guitar_di();
        let ir = vec![0.1_f32]; // attenuating delta, well below 0 dBFS
        let scaled = peak_normalize_for_convolution(&di, &ir, -1.0);
        let wet = convolve(&di, &scaled);
        let p = peak_dbfs(&wet);
        // Brought UP to -1 dBFS too (target is exact, not an upper bound).
        assert!((p - (-1.0)).abs() < 0.05, "peak was {p:.3} dBFS");
        let _ = sr(); // silence unused-helper warn in narrow tests
    }
}
