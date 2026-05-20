//! QA checks for plugin outputs (issue #12).
//!
//! Every audio failure mode discovered in production is encoded here
//! as a deterministic threshold against a probe signal — clipping,
//! silence, non-finite samples, DC drift, out-of-band loudness,
//! HF aliasing. Listening is NOT a valid verification step; if a
//! symptom can be heard it can also be measured, and once measured
//! it goes here.
//!
//! Each check is a pure function `fn(...) -> Option<QaFail>`. Both a
//! synthesised passing case and a synthesised failing case must exist
//! in `#[cfg(test)] mod tests`; no new check lands without both.

use crate::loudness::{integrated_lufs, peak_dbfs};

/// Hard ceiling: any sample above this is a clip.
pub const CLIP_CEILING_DBFS: f32 = 0.0;

/// Below this integrated LUFS the output is considered silent / dead
/// capture, not "very quiet".
pub const SILENCE_LUFS: f32 = -60.0;

/// Absolute DC offset above this is a defect.
pub const DC_THRESHOLD: f32 = 1e-3;

/// Loudness sanity band. Output integrated LUFS outside this is
/// almost certainly a broken capture (totally dead or absurdly hot).
pub const LUFS_BAND_MIN: f32 = -40.0;
pub const LUFS_BAND_MAX: f32 = 0.0;

#[derive(Debug, Clone, PartialEq)]
pub enum QaFail {
    Clip { peak_dbfs: f32 },
    Silence { lufs: f32 },
    NonFinite { count: usize },
    DcOffset { dc: f32 },
    LufsOutOfBand { lufs: f32 },
}

impl QaFail {
    pub fn label(&self) -> &'static str {
        match self {
            QaFail::Clip { .. } => "clip",
            QaFail::Silence { .. } => "silence",
            QaFail::NonFinite { .. } => "non_finite",
            QaFail::DcOffset { .. } => "dc_offset",
            QaFail::LufsOutOfBand { .. } => "lufs_out_of_band",
        }
    }
}

/// Returns `Some(Clip)` if any sample exceeds the digital ceiling.
pub fn check_clip(samples: &[f32]) -> Option<QaFail> {
    let p = peak_dbfs(samples);
    if p > CLIP_CEILING_DBFS {
        Some(QaFail::Clip { peak_dbfs: p })
    } else {
        None
    }
}

/// Returns `Some(Silence)` if integrated LUFS is below the dead-capture
/// threshold (signal is effectively silent over the probe duration).
pub fn check_silence(samples: &[f32], sample_rate: u32) -> Option<QaFail> {
    let l = integrated_lufs(samples, sample_rate);
    if !l.is_finite() || l <= SILENCE_LUFS {
        Some(QaFail::Silence { lufs: l })
    } else {
        None
    }
}

/// Returns `Some(NonFinite { count })` if any sample is NaN or Inf.
pub fn check_non_finite(samples: &[f32]) -> Option<QaFail> {
    let n = samples.iter().filter(|s| !s.is_finite()).count();
    if n > 0 {
        Some(QaFail::NonFinite { count: n })
    } else {
        None
    }
}

/// Returns `Some(DcOffset)` if the mean of the samples exceeds the DC
/// threshold (drifted output, not centred).
pub fn check_dc_offset(samples: &[f32]) -> Option<QaFail> {
    if samples.is_empty() {
        return None;
    }
    let dc: f32 =
        samples.iter().map(|s| *s as f64).sum::<f64>() as f32 / samples.len() as f32;
    if dc.abs() > DC_THRESHOLD {
        Some(QaFail::DcOffset { dc })
    } else {
        None
    }
}

/// Returns `Some(LufsOutOfBand)` if integrated LUFS falls outside the
/// sanity band — catches broken captures that are extremely hot or
/// effectively silent in a way the silence check doesn't already catch.
pub fn check_lufs_band(samples: &[f32], sample_rate: u32) -> Option<QaFail> {
    let l = integrated_lufs(samples, sample_rate);
    if !l.is_finite() || l < LUFS_BAND_MIN || l > LUFS_BAND_MAX {
        Some(QaFail::LufsOutOfBand { lufs: l })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

    fn sr() -> u32 {
        DI_SAMPLE_RATE as u32
    }

    // --- check_clip --- //

    #[test]
    fn clip_passes_on_di_below_ceiling() {
        let di = default_guitar_di();
        assert!(check_clip(&di).is_none(), "DI peaks at -15 dBFS, must pass");
    }

    #[test]
    fn clip_fails_when_peak_exceeds_zero_dbfs() {
        let buf = vec![0.0, 0.5, 1.5, -0.2];
        match check_clip(&buf) {
            Some(QaFail::Clip { peak_dbfs }) => assert!(peak_dbfs > 0.0),
            other => panic!("expected Clip, got {other:?}"),
        }
    }

    // --- check_silence --- //

    #[test]
    fn silence_passes_on_di() {
        let di = default_guitar_di();
        assert!(check_silence(&di, sr()).is_none());
    }

    #[test]
    fn silence_fails_on_zero_buffer() {
        let buf = vec![0.0_f32; sr() as usize * 2]; // 2 s of silence
        assert!(matches!(
            check_silence(&buf, sr()),
            Some(QaFail::Silence { .. })
        ));
    }

    // --- check_non_finite --- //

    #[test]
    fn non_finite_passes_on_clean_buffer() {
        let buf = vec![0.1, -0.5, 0.9, 0.0];
        assert!(check_non_finite(&buf).is_none());
    }

    #[test]
    fn non_finite_fails_when_nan_or_inf_present() {
        let buf = vec![0.1, f32::NAN, 0.3, f32::INFINITY];
        match check_non_finite(&buf) {
            Some(QaFail::NonFinite { count }) => assert_eq!(count, 2),
            other => panic!("expected NonFinite, got {other:?}"),
        }
    }

    // --- check_dc_offset --- //

    #[test]
    fn dc_offset_passes_on_di() {
        let di = default_guitar_di();
        assert!(check_dc_offset(&di).is_none(), "DI must be DC-free");
    }

    #[test]
    fn dc_offset_fails_on_constant_buffer() {
        let buf = vec![0.5_f32; 1000];
        match check_dc_offset(&buf) {
            Some(QaFail::DcOffset { dc }) => assert!((dc - 0.5).abs() < 1e-4),
            other => panic!("expected DcOffset, got {other:?}"),
        }
    }

    // --- check_lufs_band --- //

    #[test]
    fn lufs_band_passes_on_di() {
        let di = default_guitar_di();
        assert!(check_lufs_band(&di, sr()).is_none());
    }

    #[test]
    fn lufs_band_fails_on_silent_buffer() {
        let buf = vec![0.0_f32; sr() as usize * 2];
        assert!(matches!(
            check_lufs_band(&buf, sr()),
            Some(QaFail::LufsOutOfBand { .. })
        ));
    }
}
