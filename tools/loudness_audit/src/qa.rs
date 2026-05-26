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
use realfft::RealFftPlanner;

/// Hard ceiling for LINEAR blocks (IR / cab / body): any sample above
/// this is a clip. Linear filters must not exceed digital full scale
/// — a cab IR that clips is a data defect.
pub const CLIP_CEILING_DBFS: f32 = 0.0;

/// Tolerance ceiling for NONLINEAR blocks (NAM amp/preamp/gain_pedal).
/// NAM models routinely produce brief inter-sample peaks slightly above
/// 0 dBFS that the runtime tanh limiter absorbs; flagging those as
/// clips is a false positive. Aligned with the runtime curve, which
/// stays transparent up to ~+1 dBFS.
pub const CLIP_CEILING_NONLINEAR_DBFS: f32 = 1.0;

/// DC tolerance for NONLINEAR blocks. Asymmetric clipping in a model
/// naturally produces a small DC component; an order of magnitude
/// looser than the linear threshold covers every currently-healthy NAM
/// in the catalogue while still catching genuine drift.
pub const DC_THRESHOLD_NONLINEAR: f32 = 1e-2;

/// HF-aliasing margin for NONLINEAR blocks. Distortion harmonics ARE
/// the tone; the linear margin flags legitimate harmonic content.
/// The brightest currently-healthy NAM models top out near +33 dB;
/// +35 dB tolerates them with a small safety margin while still
/// catching gross HF garbage (synthetic alias case is > 60 dB).
pub const HF_ALIASING_MARGIN_NONLINEAR_DB: f32 = 35.0;

/// Lower LUFS bound for NONLINEAR blocks. A few very-quiet captures
/// (sub-driven Mesa Boogie 290 simul-class, vintage tube drivers)
/// integrate well below the linear floor against the synthetic DI;
/// −50 LUFS covers them while still catching dead models (silence
/// check at −60 still applies as the absolute lower bound).
pub const LUFS_BAND_MIN_NONLINEAR: f32 = -50.0;

/// Below this integrated LUFS the output is considered silent / dead
/// capture, not "very quiet".
pub const SILENCE_LUFS: f32 = -60.0;

/// Body-class silence floor (issue #23). The spectral-unity makeup
/// can cut a heavily resonant pickup-emulation body IR by 30–45 dB,
/// dropping the convolved signal to −60 to −70 LUFS while the
/// underlying capture is still a real (if narrow) filter. −75 leaves
/// 15 LU of margin under the standard floor before flagging.
pub const SILENCE_LUFS_BODY: f32 = -75.0;

/// Absolute DC offset above this is a defect.
pub const DC_THRESHOLD: f32 = 1e-3;

/// Loudness sanity band. Output integrated LUFS outside this is
/// almost certainly a broken capture (totally dead or absurdly hot).
/// Floor unified at −50 LUFS across classes (issue #23): the
/// spectral-unity IR audit produces aggressive negative makeup
/// (−10 to −30 dB) that drops convolved LUFS well below the old
/// −40 LinearCab floor without indicating defective content. The
/// SILENCE check at −60 still flags truly dead captures.
pub const LUFS_BAND_MIN: f32 = -50.0;
pub const LUFS_BAND_MAX: f32 = 0.0;

/// Lower LUFS bound for acoustic-body IRs (issue #21). Body captures
/// are pickup-emulation filters — narrow-band by design, naturally
/// quieter than electric cab IRs whose response sits in the
/// guitar-relevant midrange. A handful of pickup-emulation flavors
/// (`*_hfn`, `*_matcheq`, `*_bld`) integrate around −42 to −46 LUFS
/// against the synthetic DI without being defective. −50 LUFS covers
/// them while the absolute silence check at −60 still catches dead
/// captures.
pub const LUFS_BAND_MIN_BODY: f32 = -50.0;

/// Lower edge (Hz) of the "alias-likely" band used by `check_hf_aliasing`.
/// Energy here that exceeds the probe's energy by more than the margin
/// is the signature of imaging artefacts (linear-resample images,
/// numerical garbage near Nyquist). Above-this-band nonlinear distortion
/// harmonics are tolerated by tuning the margin generously.
pub const ALIAS_BAND_START_HZ: f32 = 18_000.0;

/// dB margin allowed for output energy in the alias band above probe
/// energy in the same band, for LINEAR blocks. Acoustic body IRs are
/// inherently brighter than electric cab IRs (more legitimate energy
/// above 18 kHz from string transients), which sets the floor at ~15
/// dB; 16 dB leaves a 1 dB safety margin while still failing imaging
/// artefacts cleanly (synthetic alias case is > 30 dB).
pub const HF_ALIASING_MARGIN_DB: f32 = 16.0;

#[derive(Debug, Clone, PartialEq)]
pub enum QaFail {
    Clip { peak_dbfs: f32 },
    Silence { lufs: f32 },
    NonFinite { count: usize },
    DcOffset { dc: f32 },
    LufsOutOfBand { lufs: f32 },
    HfAliasing { delta_db: f32, band_start_hz: f32 },
    SpectralPeak { peak_db: f32 },
}

impl QaFail {
    pub fn label(&self) -> &'static str {
        match self {
            QaFail::Clip { .. } => "clip",
            QaFail::Silence { .. } => "silence",
            QaFail::NonFinite { .. } => "non_finite",
            QaFail::DcOffset { .. } => "dc_offset",
            QaFail::LufsOutOfBand { .. } => "lufs_out_of_band",
            QaFail::HfAliasing { .. } => "hf_aliasing",
            QaFail::SpectralPeak { .. } => "spectral_peak",
        }
    }
}

/// Minimum FFT size for spectral-magnitude analysis. 16384 bins at
/// 48 kHz give ~2.9 Hz resolution, fine enough to catch the narrow
/// resonant peaks (≈ 50–100 Hz wide) that drive cab/body IR misuse.
const SPECTRAL_FFT_MIN: usize = 16_384;

/// Peak magnitude `max |H(f)|` of `ir` in dB. The IR is treated as a
/// linear filter; its DFT magnitude is the gain it imposes on a sine
/// at each bin. Zero-padded to at least `SPECTRAL_FFT_MIN` so a
/// narrow resonance is not undersampled in frequency.
///
/// Returns `f32::NEG_INFINITY` for an empty or all-zero IR (`log10(0)`
/// is undefined; the caller treats this as "no useful response" and
/// must not divide by it).
pub fn peak_spectral_magnitude_db(ir: &[f32], _sample_rate: u32) -> f32 {
    if ir.is_empty() {
        return f32::NEG_INFINITY;
    }
    let n = ir.len().next_power_of_two().max(SPECTRAL_FFT_MIN);
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut buf = fft.make_input_vec();
    buf[..ir.len()].copy_from_slice(ir);
    let mut spec = fft.make_output_vec();
    fft.process(&mut buf, &mut spec).unwrap();
    let mut max_mag_sq: f64 = 0.0;
    for c in spec.iter() {
        let m = (c.re as f64).powi(2) + (c.im as f64).powi(2);
        if m > max_mag_sq {
            max_mag_sq = m;
        }
    }
    if max_mag_sq <= 0.0 {
        return f32::NEG_INFINITY;
    }
    (10.0 * max_mag_sq.log10()) as f32
}

/// Spectral-magnitude threshold for cab/body IRs (issue #23). The
/// audit writes `output_gain_db = -peak_spectral_magnitude_db(ir)` so
/// the IR's worst-case frequency response is unity after makeup. This
/// check rejects any capture whose post-makeup spectral peak still
/// exceeds the ceiling — guards against a future audit run that uses
/// the wrong formula or skips the IR class entirely.
pub const SPECTRAL_PEAK_CEILING_DB: f32 = 0.5;

/// Returns `Some(SpectralPeak)` if `max |H(f)|` of `ir_post_makeup`
/// exceeds `ceiling_db`. Caller is expected to pre-scale the IR by
/// the manifest's `output_gain_db` so the assertion is on the IR the
/// engine actually convolves with.
pub fn check_spectral_peak_with(
    ir_post_makeup: &[f32],
    sample_rate: u32,
    ceiling_db: f32,
) -> Option<QaFail> {
    let p = peak_spectral_magnitude_db(ir_post_makeup, sample_rate);
    if p > ceiling_db {
        Some(QaFail::SpectralPeak { peak_db: p })
    } else {
        None
    }
}

/// Linear-block (cab/body) spectral peak check with the standard
/// ceiling.
pub fn check_spectral_peak(ir_post_makeup: &[f32], sample_rate: u32) -> Option<QaFail> {
    check_spectral_peak_with(ir_post_makeup, sample_rate, SPECTRAL_PEAK_CEILING_DB)
}

/// Total energy (sum of squared magnitudes) in bins whose centre
/// frequency is `>= band_start_hz`. Energy floor returned as a small
/// positive value so the dB ratio in `check_hf_aliasing` is finite.
fn band_energy_above(samples: &[f32], sample_rate: u32, band_start_hz: f32) -> f64 {
    if samples.is_empty() {
        return 1e-30;
    }
    let n = samples.len().next_power_of_two().max(2);
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut buf = fft.make_input_vec();
    buf[..samples.len()].copy_from_slice(samples);
    let mut spec = fft.make_output_vec();
    fft.process(&mut buf, &mut spec).unwrap();
    let bin_hz = sample_rate as f32 / n as f32;
    let mut e: f64 = 0.0;
    for (k, c) in spec.iter().enumerate() {
        let f = k as f32 * bin_hz;
        if f >= band_start_hz {
            e += (c.re as f64).powi(2) + (c.im as f64).powi(2);
        }
    }
    e.max(1e-30)
}

/// Returns `Some(HfAliasing)` if the output has more energy in the
/// alias-likely band than the probe by more than the given margin.
/// Catches resampler imaging and similar HF garbage on linear blocks;
/// nonlinear blocks must use a wider margin to allow legitimate
/// distortion harmonics.
pub fn check_hf_aliasing_with(
    probe: &[f32],
    out: &[f32],
    sample_rate: u32,
    margin_db: f32,
) -> Option<QaFail> {
    let e_in = band_energy_above(probe, sample_rate, ALIAS_BAND_START_HZ);
    let e_out = band_energy_above(out, sample_rate, ALIAS_BAND_START_HZ);
    let delta_db = (10.0 * (e_out / e_in).log10()) as f32;
    if delta_db > margin_db {
        Some(QaFail::HfAliasing {
            delta_db,
            band_start_hz: ALIAS_BAND_START_HZ,
        })
    } else {
        None
    }
}

/// Linear-block HF aliasing check (strict +12 dB margin).
pub fn check_hf_aliasing(probe: &[f32], out: &[f32], sample_rate: u32) -> Option<QaFail> {
    check_hf_aliasing_with(probe, out, sample_rate, HF_ALIASING_MARGIN_DB)
}

/// Returns `Some(Clip)` if any sample exceeds the given ceiling.
pub fn check_clip_with(samples: &[f32], ceiling_dbfs: f32) -> Option<QaFail> {
    let p = peak_dbfs(samples);
    if p > ceiling_dbfs {
        Some(QaFail::Clip { peak_dbfs: p })
    } else {
        None
    }
}

/// Linear-block clip check (strict 0 dBFS).
pub fn check_clip(samples: &[f32]) -> Option<QaFail> {
    check_clip_with(samples, CLIP_CEILING_DBFS)
}

/// Returns `Some(Silence)` if integrated LUFS is below the dead-capture
/// threshold (signal is effectively silent over the probe duration).
pub fn check_silence(samples: &[f32], sample_rate: u32) -> Option<QaFail> {
    check_silence_with(samples, sample_rate, SILENCE_LUFS)
}

/// Class-aware silence check with an explicit threshold. Use
/// `SILENCE_LUFS_BODY` for body IRs whose spectral-unity makeup can
/// legitimately drive convolved LUFS below the standard floor.
pub fn check_silence_with(
    samples: &[f32],
    sample_rate: u32,
    threshold_db: f32,
) -> Option<QaFail> {
    let l = integrated_lufs(samples, sample_rate);
    if !l.is_finite() || l <= threshold_db {
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

/// Returns `Some(DcOffset)` if the mean of the samples exceeds the
/// given DC threshold (drifted output, not centred).
pub fn check_dc_offset_with(samples: &[f32], threshold: f32) -> Option<QaFail> {
    if samples.is_empty() {
        return None;
    }
    let dc: f32 =
        samples.iter().map(|s| *s as f64).sum::<f64>() as f32 / samples.len() as f32;
    if dc.abs() > threshold {
        Some(QaFail::DcOffset { dc })
    } else {
        None
    }
}

/// Linear-block DC check (strict 1e-3).
pub fn check_dc_offset(samples: &[f32]) -> Option<QaFail> {
    check_dc_offset_with(samples, DC_THRESHOLD)
}

/// Returns `Some(LufsOutOfBand)` if integrated LUFS falls outside the
/// given sanity band — catches broken captures that are extremely hot
/// or effectively silent in a way the silence check doesn't already
/// catch.
pub fn check_lufs_band_with(
    samples: &[f32],
    sample_rate: u32,
    min: f32,
    max: f32,
) -> Option<QaFail> {
    let l = integrated_lufs(samples, sample_rate);
    if !l.is_finite() || l < min || l > max {
        Some(QaFail::LufsOutOfBand { lufs: l })
    } else {
        None
    }
}

/// Linear-block LUFS sanity band (strict −40..0).
pub fn check_lufs_band(samples: &[f32], sample_rate: u32) -> Option<QaFail> {
    check_lufs_band_with(samples, sample_rate, LUFS_BAND_MIN, LUFS_BAND_MAX)
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

    // --- check_hf_aliasing --- //

    #[test]
    fn hf_aliasing_passes_when_output_equals_probe() {
        let di = default_guitar_di();
        assert!(check_hf_aliasing(&di, &di, sr()).is_none());
    }

    #[test]
    fn hf_aliasing_fails_when_near_nyquist_tone_is_added() {
        let di = default_guitar_di();
        // Synthesise an output = probe + strong sinusoid at 20 kHz.
        // The probe itself has very little energy at 20 kHz; this added
        // tone reproduces the signature of a resampler-imaging artefact.
        let two_pi = std::f32::consts::TAU;
        let out: Vec<f32> = di
            .iter()
            .enumerate()
            .map(|(i, &s)| s + 0.3 * (two_pi * 20_000.0 * (i as f32) / sr() as f32).sin())
            .collect();
        match check_hf_aliasing(&di, &out, sr()) {
            Some(QaFail::HfAliasing { delta_db, .. }) => {
                assert!(delta_db > HF_ALIASING_MARGIN_DB, "delta was {delta_db}");
            }
            other => panic!("expected HfAliasing, got {other:?}"),
        }
    }

    // --- peak_spectral_magnitude_db --- //

    #[test]
    fn spectral_peak_of_unit_delta_is_zero_db() {
        // A unit impulse is a flat 0 dB filter at every frequency.
        let ir = vec![1.0_f32];
        let p = peak_spectral_magnitude_db(&ir, sr());
        assert!(p.abs() < 1e-3, "peak was {p:.6} dB");
    }

    #[test]
    fn spectral_peak_of_scaled_delta_matches_scale() {
        // A delta scaled by 2× is +6 dB flat across the spectrum.
        let ir = vec![2.0_f32];
        let p = peak_spectral_magnitude_db(&ir, sr());
        assert!((p - 6.0206).abs() < 1e-3, "peak was {p:.6} dB");
    }

    #[test]
    fn spectral_peak_finds_resonant_bump() {
        // Two-tap IR with constructive DC: [1, 1] sums to 2 at DC → +6 dB.
        // The DFT of [1, 1, 0, ..., 0] is X[k] = 1 + e^(-j 2π k / N).
        // At k=0 → 2 (DC sum). Confirms the peak finder picks the
        // maximum-magnitude bin and reports it in dB.
        let ir = vec![1.0_f32, 1.0];
        let p = peak_spectral_magnitude_db(&ir, sr());
        assert!((p - 6.0206).abs() < 0.01, "peak was {p:.6} dB");
    }

    // --- check_spectral_peak --- //

    #[test]
    fn spectral_peak_passes_when_max_h_within_ceiling() {
        // Unit delta = 0 dB max, ceiling 0.5 dB → must pass.
        let ir = vec![1.0_f32];
        assert!(check_spectral_peak(&ir, sr()).is_none());
    }

    #[test]
    fn spectral_peak_fails_when_max_h_exceeds_ceiling() {
        // 4× delta = +12.04 dB → way above the 0.5 dB ceiling.
        let ir = vec![4.0_f32];
        match check_spectral_peak(&ir, sr()) {
            Some(QaFail::SpectralPeak { peak_db }) => {
                assert!(peak_db > SPECTRAL_PEAK_CEILING_DB, "peak was {peak_db:.3} dB");
            }
            other => panic!("expected SpectralPeak, got {other:?}"),
        }
    }
}
