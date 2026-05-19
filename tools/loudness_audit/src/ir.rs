//! IR (.wav) loading + FFT convolution for cab/body loudness audit.
//! LUFS of a linear convolution is implementation-independent, so this
//! self-contained convolver is loudness-correct without depending on
//! the runtime IR crate.

use anyhow::{bail, Context, Result};
use realfft::RealFftPlanner;
use std::path::Path;

/// Full linear convolution via a single zero-padded FFT. IR + DI are
/// short enough (DI a few sec @ 48k, IR ≤ a few k taps) for one
/// transform; output length = sig + ir − 1.
pub fn convolve(sig: &[f32], ir: &[f32]) -> Vec<f32> {
    if sig.is_empty() || ir.is_empty() {
        return Vec::new();
    }
    let n_lin = sig.len() + ir.len() - 1;
    let n = n_lin.next_power_of_two();
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let ifft = planner.plan_fft_inverse(n);

    let mut a = fft.make_input_vec();
    let mut b = fft.make_input_vec();
    a[..sig.len()].copy_from_slice(sig);
    b[..ir.len()].copy_from_slice(ir);

    let mut sa = fft.make_output_vec();
    let mut sb = fft.make_output_vec();
    fft.process(&mut a, &mut sa).unwrap();
    fft.process(&mut b, &mut sb).unwrap();

    for (x, y) in sa.iter_mut().zip(sb.iter()) {
        *x *= *y;
    }
    let mut out = ifft.make_output_vec();
    ifft.process(&mut sa, &mut out).unwrap();

    let scale = 1.0 / n as f32;
    out.truncate(n_lin);
    out.iter_mut().for_each(|v| *v *= scale);
    out
}

/// Loads a mono IR `.wav` as `f32` samples normalised to [-1, 1],
/// resampled to 48 kHz if needed. Stereo files are downmixed (mean).
pub fn load_wav_ir(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    let chans = spec.channels as usize;
    if chans == 0 {
        bail!("{}: zero channels", path.display());
    }
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.samples::<f32>().collect::<Result<_, _>>()?
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };
    // Interleaved -> mono mean.
    let mono: Vec<f32> = raw
        .chunks(chans)
        .map(|f| f.iter().sum::<f32>() / chans as f32)
        .collect();
    Ok(resample_linear(&mono, spec.sample_rate, 48_000))
}

/// Linear-interpolation resample. Adequate for an integrated-LUFS
/// measurement of a short IR (tone error is irrelevant to the loudness
/// number; correctness over fidelity, no silent skip on 44.1k IRs).
pub fn resample_linear(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let out_len = (x.len() as f64 * ratio).round() as usize;
    let mut y = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let frac = (src - i0 as f64) as f32;
        let a = x.get(i0).copied().unwrap_or(0.0);
        let b = x.get(i0 + 1).copied().unwrap_or(a);
        y.push(a + (b - a) * frac);
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_known_wav_mono_normalised() {
        // Build a 4-sample 48k mono i16 wav in a temp file.
        let dir = std::env::temp_dir().join("irtest_load");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for v in [0i16, 16384, -16384, 32767] {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let got = load_wav_ir(&p).unwrap();
        assert_eq!(got.len(), 4);
        assert!((got[1] - 0.5).abs() < 1e-3);
        assert!((got[3] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn resample_44k_to_48k_scales_length_and_keeps_endpoints() {
        let x: Vec<f32> = (0..441).map(|i| i as f32 / 440.0).collect();
        let y = resample_linear(&x, 44_100, 48_000);
        // length ~ ceil(441 * 48000/44100) = 480
        assert!((y.len() as i32 - 480).abs() <= 1, "len was {}", y.len());
        assert!((y[0] - 0.0).abs() < 1e-4);
        assert!((*y.last().unwrap() - 1.0).abs() < 2e-2);
    }

    #[test]
    fn resample_noop_when_rates_equal() {
        let x = vec![0.1, 0.2, 0.3];
        assert_eq!(resample_linear(&x, 48_000, 48_000), x);
    }

    #[test]
    fn delta_ir_is_identity() {
        let sig = vec![0.1, -0.4, 0.7, 0.2, -0.9];
        let ir = vec![1.0_f32];
        let y = convolve(&sig, &ir);
        for (a, b) in sig.iter().zip(y.iter()) {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
    }

    #[test]
    fn scaled_delta_scales_signal() {
        let sig = vec![0.1, -0.4, 0.7, 0.2, -0.9];
        let ir = vec![0.5_f32]; // -6 dB
        let y = convolve(&sig, &ir);
        for (a, b) in sig.iter().zip(y.iter()) {
            assert!((a * 0.5 - b).abs() < 1e-5);
        }
    }

    #[test]
    fn matches_naive_convolution() {
        let sig: Vec<f32> = (0..200).map(|i| (i as f32 * 0.3).sin()).collect();
        let ir: Vec<f32> = (0..37).map(|i| (i as f32 * 0.11).cos() * 0.2).collect();
        let fast = convolve(&sig, &ir);
        let mut naive = vec![0.0_f32; sig.len() + ir.len() - 1];
        for (i, s) in sig.iter().enumerate() {
            for (j, h) in ir.iter().enumerate() {
                naive[i + j] += s * h;
            }
        }
        assert_eq!(fast.len(), naive.len());
        for (a, b) in fast.iter().zip(naive.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }
}
