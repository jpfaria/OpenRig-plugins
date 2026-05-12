//! Shared loudness/peak helpers + a copy of the runtime's
//! `output_limiter` so the audit and the catalog test see the same
//! signal the user hears.
//!
//! `output_limiter` MUST stay byte-identical with
//! `crates/engine/src/runtime_dsp.rs::output_limiter` in OpenRig.
//! If the runtime curve changes, this copy needs to follow — there's
//! a unit test that pins a few characteristic samples to catch drift.

/// Integrated LUFS via BS.1770. Mono signal — for stereo, average L+R
/// loudness (broadcast-mono guitar in OpenRig is L=R, so the same).
pub fn integrated_lufs(samples: &[f32], sample_rate: u32) -> f32 {
    let mut meter = bs1770::ChannelLoudnessMeter::new(sample_rate);
    meter.push(samples.iter().copied());
    let windows = meter.into_100ms_windows();
    let gated = bs1770::gated_mean(windows.as_ref());
    gated.loudness_lkfs()
}

pub fn peak_dbfs(samples: &[f32]) -> f32 {
    let peak = samples.iter().fold(0.0_f32, |a, s| a.max(s.abs()));
    if peak == 0.0 {
        -120.0
    } else {
        20.0 * peak.log10()
    }
}

/// Mirror of `crates/engine/src/runtime_dsp.rs::output_limiter`.
/// Soft tanh saturation above |s| > 0.95.
#[inline]
pub fn output_limiter(sample: f32) -> f32 {
    if sample.abs() < 0.95 {
        sample
    } else {
        sample.tanh()
    }
}

pub fn apply_output_limiter(samples: &mut [f32]) {
    for s in samples.iter_mut() {
        *s = output_limiter(*s);
    }
}

#[inline]
pub fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_passes_low_level_unchanged() {
        for s in [0.0, 0.1, 0.5, 0.94, -0.94] {
            assert_eq!(output_limiter(s), s, "should be transparent at {s}");
        }
    }

    #[test]
    fn limiter_saturates_above_threshold() {
        // Match runtime curve: tanh above 0.95.
        assert!((output_limiter(1.0) - 1.0_f32.tanh()).abs() < 1e-6);
        assert!((output_limiter(2.0) - 2.0_f32.tanh()).abs() < 1e-6);
        assert!((output_limiter(-1.5) - (-1.5_f32).tanh()).abs() < 1e-6);
    }

    #[test]
    fn db_to_lin_round_trips() {
        let lin = db_to_lin(6.0);
        let back = 20.0 * lin.log10();
        assert!((back - 6.0).abs() < 1e-4);
    }

    #[test]
    fn peak_of_silence_is_floor() {
        let buf = vec![0.0_f32; 100];
        assert_eq!(peak_dbfs(&buf), -120.0);
    }

    #[test]
    fn peak_of_full_scale_is_zero_dbfs() {
        let mut buf = vec![0.0_f32; 100];
        buf[50] = 1.0;
        assert!((peak_dbfs(&buf) - 0.0).abs() < 1e-4);
    }
}
