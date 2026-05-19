//! IR (.wav) loading + FFT convolution for cab/body loudness audit.
//! LUFS of a linear convolution is implementation-independent, so this
//! self-contained convolver is loudness-correct without depending on
//! the runtime IR crate.

use anyhow::{bail, Context, Result};
use std::path::Path;

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

pub fn resample_linear(_x: &[f32], _from: u32, _to: u32) -> Vec<f32> {
    unimplemented!()
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
}
