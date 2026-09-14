//! Output-level policy for NAM and IR blocks (issue #143).
//!
//! Every block ships the `output_gain_db` that puts its reference output
//! at the **maximum level that does not clip**: its peak lands on
//! [`TARGET_PEAK_DBFS`], the threshold of the engine's chain-end
//! brick-wall limiter, so the limiter never engages on the reference
//! signal. Gains are signed (quiet blocks are boosted, hot blocks cut) and
//! bounded to the range of OpenRig's Output knob, which is where the value
//! lands (the block factory seeds `output_db` from it).
//!
//! What "reference output" means depends on where the block sits:
//!
//! - NAM (amp / preamp / gain_pedal): the synthetic DI through the model.
//!   The engine applies ONE manifest-level value to every capture, so the
//!   writer uses the loudest capture — no capture may clip.
//! - IR cab: a cab is fed by an amp, not by the raw DI. It is probed with
//!   amp-level signals already at the target (a clean and a saturated DI),
//!   and the gain keeps the worst of them at the target.
//! - IR body: an acoustic body is fed by the raw DI (piezo), so it is
//!   probed with the DI itself.
//!
//! Supersedes the boost-only LUFS rule (#4), which left the headroom of
//! 299/343 NAM plugins unused, and the spectral-unity IR rule (#23), which
//! sized every cab/body for a pure sine at its worst resonance (median
//! −21 dB, 586 captures beyond the knob range).

use crate::ir::convolve;
use crate::loudness::{db_to_lin, peak_dbfs};

/// Peak (dBFS) every block's reference output is normalised to: the
/// engine brick-wall limiter threshold (`limiter::limit_default`).
pub const TARGET_PEAK_DBFS: f32 = -1.0;

/// Range of OpenRig's Output knob (`output_db`, NAM and IR alike). A gain
/// outside it cannot be represented by the block, so the writer clamps.
pub const OUTPUT_KNOB_MIN_DB: f32 = -24.0;
pub const OUTPUT_KNOB_MAX_DB: f32 = 24.0;

/// Highest pre-makeup IR reference peak the Output knob can still pull
/// down to the target. A hotter `.wav` is scaled down by `qa_fix`
/// ([`ir_knob_fit_scale`]) — level stays the manifest's job, but only
/// within what the knob can represent.
pub const IR_LEVEL_PEAK_CEILING_DBFS: f32 = TARGET_PEAK_DBFS - OUTPUT_KNOB_MIN_DB;

/// Allowed distance (dB) between a block's post-gain reference peak and
/// the target before `qa::check_level` fails.
pub const LEVEL_TOLERANCE_DB: f32 = 0.1;

/// Drive (dB) into the `tanh` stage that turns the DI into the saturated
/// amp-level cab probe: deep enough to square the waveform off like a
/// high-gain amp, which packs far more energy per peak than a clean DI.
pub const CAB_PROBE_DRIVE_DB: f32 = 30.0;

/// Which reference signal an IR is levelled against.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum IrRole {
    /// Cabinet, fed by an amp: probed with [`amp_level_probes`].
    Cab,
    /// Acoustic body, fed by the raw DI.
    Body,
}

/// Reference peak (dBFS) of a rendered block output: the WHOLE signal,
/// opening attack included — the same span the gate's clip check sees.
pub fn level_peak_dbfs(samples: &[f32]) -> f32 {
    peak_dbfs(samples)
}

/// `output_gain_db` that moves a reference peak onto the target, clamped
/// to the Output knob range.
pub fn target_gain_db(measured_peak_dbfs: f32) -> f32 {
    (TARGET_PEAK_DBFS - measured_peak_dbfs).clamp(OUTPUT_KNOB_MIN_DB, OUTPUT_KNOB_MAX_DB)
}

/// Amp-level cab probes, both peaking at the target: the clean DI and a
/// `tanh`-saturated DI (high-gain amp output).
pub fn amp_level_probes(di: &[f32]) -> [Vec<f32>; 2] {
    let drive = db_to_lin(CAB_PROBE_DRIVE_DB);
    let driven: Vec<f32> = di.iter().map(|s| (s * drive).tanh()).collect();
    [normalised_to_target(di), normalised_to_target(&driven)]
}

/// Reference peak (dBFS) of an IR before any makeup: the worst probe
/// peak for a cab, the DI peak for a body.
pub fn ir_level_peak_dbfs(ir: &[f32], di: &[f32], role: IrRole) -> f32 {
    match role {
        IrRole::Body => peak_dbfs(&convolve(di, ir)),
        IrRole::Cab => amp_level_probes(di)
            .iter()
            .map(|p| peak_dbfs(&convolve(p, ir)))
            .fold(f32::NEG_INFINITY, f32::max),
    }
}

/// Linear scale (≤ 1) that brings an IR's reference peak down to
/// [`IR_LEVEL_PEAK_CEILING_DBFS`]; 1.0 when it already fits the knob.
pub fn ir_knob_fit_scale(ir: &[f32], di: &[f32], role: IrRole) -> f32 {
    let peak = ir_level_peak_dbfs(ir, di, role);
    if !peak.is_finite() || peak <= IR_LEVEL_PEAK_CEILING_DBFS {
        return 1.0;
    }
    db_to_lin(IR_LEVEL_PEAK_CEILING_DBFS - peak)
}

fn normalised_to_target(x: &[f32]) -> Vec<f32> {
    let scale = db_to_lin(TARGET_PEAK_DBFS - peak_dbfs(x));
    x.iter().map(|s| s * scale).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

    const SR: u32 = DI_SAMPLE_RATE as u32;

    fn crest_db(x: &[f32]) -> f32 {
        let rms = (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        peak_dbfs(x) - 20.0 * rms.log10()
    }

    #[test]
    fn quiet_block_is_boosted_to_target() {
        assert!((target_gain_db(-13.0) - 12.0).abs() < 1e-4);
    }

    #[test]
    fn hot_block_is_cut_to_target() {
        assert!((target_gain_db(5.0) - -6.0).abs() < 1e-4);
    }

    #[test]
    fn gain_is_clamped_to_output_knob_range() {
        assert_eq!(target_gain_db(-60.0), OUTPUT_KNOB_MAX_DB);
        assert_eq!(target_gain_db(40.0), OUTPUT_KNOB_MIN_DB);
    }

    #[test]
    fn level_peak_counts_the_opening_attack() {
        // The DI's first pluck attacks at t=0 and the gate's clip check
        // sees it, so the level measure must too (diezel_hagen clipped at
        // +1.08 dBFS when the first 100 ms were skipped).
        let mut x = vec![0.0_f32; SR as usize];
        x[10] = 1.0;
        x[SR as usize / 2] = 0.5;
        assert!(level_peak_dbfs(&x).abs() < 1e-3);
    }

    #[test]
    fn amp_level_probes_both_peak_at_target() {
        for p in amp_level_probes(&default_guitar_di()) {
            assert!((peak_dbfs(&p) - TARGET_PEAK_DBFS).abs() < 0.01);
        }
    }

    #[test]
    fn saturated_probe_is_denser_than_clean() {
        let [clean, driven] = amp_level_probes(&default_guitar_di());
        assert!(crest_db(&driven) + 6.0 < crest_db(&clean));
    }

    #[test]
    fn unit_ir_leaves_cab_probe_at_target_and_body_at_di_peak() {
        let di = default_guitar_di();
        let delta = [1.0_f32];
        assert!((ir_level_peak_dbfs(&delta, &di, IrRole::Cab) - TARGET_PEAK_DBFS).abs() < 0.01);
        assert!((ir_level_peak_dbfs(&delta, &di, IrRole::Body) - peak_dbfs(&di)).abs() < 0.01);
    }

    #[test]
    fn hot_ir_is_scaled_so_its_makeup_lands_on_the_knob_floor() {
        let di = default_guitar_di();
        let hot = [32.0_f32]; // +30.1 dB: needs ~-31 dB, beyond the knob
        let s = ir_knob_fit_scale(&hot, &di, IrRole::Cab);
        let fitted = [hot[0] * s];
        let peak = ir_level_peak_dbfs(&fitted, &di, IrRole::Cab);
        assert!((peak - IR_LEVEL_PEAK_CEILING_DBFS).abs() < 0.01, "peak {peak}");
        assert!((target_gain_db(peak) - OUTPUT_KNOB_MIN_DB).abs() < 0.01);
    }

    #[test]
    fn ir_within_knob_range_is_left_alone() {
        let di = default_guitar_di();
        assert_eq!(ir_knob_fit_scale(&[4.0_f32], &di, IrRole::Cab), 1.0);
        assert_eq!(ir_knob_fit_scale(&[1.0_f32], &di, IrRole::Body), 1.0);
    }

    #[test]
    fn cab_level_scales_with_ir_gain() {
        let di = default_guitar_di();
        let peak = ir_level_peak_dbfs(&[2.0_f32], &di, IrRole::Cab);
        assert!((peak - (TARGET_PEAK_DBFS + 6.0206)).abs() < 0.01);
    }
}
