//! `loudness_audit` — writes `output_gain_db` into each plugin
//! `manifest.yaml`, measured with a deterministic synthetic guitar DI.
//! Serves both backends:
//!
//! - NAM (`amp`/`preamp`/`gain_pedal`): the DI is run through the
//!   `.nam` model. One `output_gain_db` per manifest.
//! - IR (`cab`/`body`): the DI is convolved through each capture's
//!   `.wav`. One `output_gain_db` PER capture, since each `.wav` in
//!   the grid changes level differently (issue #8).
//!
//! The correction is static manifest metadata: running this binary
//! before a release refreshes the persisted offset so the app applies
//! it as a constant gain.
//!
//! Strategy — UNITY insertion correction (signed, issue #9):
//!   gain = LUFS_in − LUFS_out
//!     LUFS_in  = integrated LUFS of the dry DI
//!     LUFS_out = integrated LUFS after the block (model / IR)
//!
//! - Targets UNITY: the block's output ends at the same loudness as
//!   its input — toggling the block does not change perceived volume.
//! - SIGNED, not boost-only: a block that amplifies (LUFS_out >
//!   LUFS_in) gets a NEGATIVE gain (it is brought back down); a block
//!   that attenuates gets a positive one. This is what stops the
//!   chained-gain blow-up of the old hot boost-only model.
//! - True-peak safety only: the positive side is capped so the
//!   correction itself never pushes the peak past 0 dBFS; never a hot
//!   loudness target.
//!
//! Usage:
//!
//!     cargo run --release -p loudness-audit -- \
//!         /path/to/OpenRig-plugins/plugins/source/<nam|ir>
//!
//! Writing preserves YAML ordering/spacing — it only replaces or
//! inserts the `output_gain_db:` line (per capture for IR).

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use nam::processor::{close_model_diag, nam_process, open_model_diag};
use loudness_audit::ir::{convolve, load_wav_ir};
use loudness_audit::loudness::{
    apply_output_limiter, db_to_lin, integrated_lufs, peak_dbfs,
};
use loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

/// True-peak safety ceiling in dBFS. The unity correction is only
/// capped on the POSITIVE (boost) side so the makeup itself never
/// pushes the post-gain peak past digital full scale. This is a
/// clip guard, NOT a loudness target — the old +3 dBFS hot ceiling
/// (which leaned on a runtime limiter and let chained gains blow up)
/// is gone.
const PEAK_CEILING_DBFS: f32 = 0.0;

/// Runaway guard for the boost direction only. A broken near-silent
/// capture would otherwise demand an enormous positive gain; 30 dB
/// caps that. Attenuation (negative gain) is intentionally unbounded:
/// a very hot block must be brought all the way back to unity.
const MAX_GAIN_DB: f32 = 30.0;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: loudness_audit <plugins-root>");
        eprintln!();
        eprintln!("Expects a directory whose immediate children are NAM plugin");
        eprintln!("packages (each carrying its own manifest.yaml + captures/).");
        std::process::exit(2);
    }
    let root = PathBuf::from(&args[1]);
    if !root.is_dir() {
        bail!("not a directory: {}", root.display());
    }

    let di = default_guitar_di();

    eprintln!("DI: {} samples @ {} Hz", di.len(), DI_SAMPLE_RATE as u32);
    eprintln!(
        "unity insertion correction (signed); true-peak cap {PEAK_CEILING_DBFS:+.2} dBFS, boost cap {MAX_GAIN_DB:+.0} dB"
    );
    eprintln!();
    eprintln!(
        "{:<48} {:>8} {:>8} {:>8} {:>8}",
        "plugin", "lufs", "peak", "want_lu", "applied"
    );

    let mut entries: Vec<PathBuf> = fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();

    let mut audited = 0usize;
    let mut skipped = 0usize;
    for plugin_dir in entries {
        let manifest_path = plugin_dir.join("manifest.yaml");
        if !manifest_path.is_file() {
            continue;
        }
        match audit_plugin(&plugin_dir, &manifest_path, &di) {
            Ok(report) => {
                let label = plugin_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<?>");
                eprintln!(
                    "{:<48} {:>+7.2}  {:>+7.2}  {:>+7.2}  {:>+7.2}",
                    label,
                    report.measured_lufs,
                    report.measured_peak_dbfs,
                    report.want_for_lufs_db,
                    report.applied_gain_db
                );
                audited += 1;
            }
            Err(e) => {
                let label = plugin_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<?>");
                eprintln!("SKIP {label}: {e}");
                skipped += 1;
            }
        }
    }

    eprintln!();
    eprintln!("audited {audited} plugins, skipped {skipped}");
    Ok(())
}

struct AuditReport {
    measured_lufs: f32,
    measured_peak_dbfs: f32,
    want_for_lufs_db: f32,
    applied_gain_db: f32,
}

fn audit_plugin(
    plugin_dir: &Path,
    manifest_path: &Path,
    di: &[f32],
) -> Result<AuditReport> {
    let raw = fs::read_to_string(manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let block_type = manifest_block_type(&raw).unwrap_or_else(|| "<unknown>".into());
    if !is_loudness_normalisable(&block_type) {
        bail!("type `{block_type}` is not loudness-normalised");
    }
    if matches!(block_type.as_str(), "cab" | "body") {
        return audit_ir_plugin(plugin_dir, manifest_path, &raw, di);
    }
    let first_capture = first_capture_file(&raw)
        .ok_or_else(|| anyhow!("no `captures:[].file` entry in manifest"))?;
    let model_path = plugin_dir.join(&first_capture);
    let model_path_str = model_path
        .to_str()
        .ok_or_else(|| anyhow!("non-utf8 capture path: {model_path:?}"))?;

    let model = open_model_diag(model_path_str)
        .with_context(|| format!("failed to load {model_path_str}"))?;
    let mut output = vec![0.0_f32; di.len()];
    unsafe {
        nam_process(model, di, &mut output);
        close_model_diag(model);
    }

    // Signed unity correction: bring the model output back to the
    // loudness of its own input (the dry DI). Negative for models
    // that amplify, positive for those that attenuate.
    let lufs_in = integrated_lufs(di, DI_SAMPLE_RATE as u32);
    let measured_lufs = integrated_lufs(&output, DI_SAMPLE_RATE as u32);
    let measured_peak_dbfs = peak_dbfs(&output);

    let want_for_lufs = lufs_in - measured_lufs;
    // Only the boost side is bounded: true-peak guard so the makeup
    // can't clip, and a runaway cap. Attenuation is unbounded.
    let peak_headroom = PEAK_CEILING_DBFS - measured_peak_dbfs;
    let applied = want_for_lufs.min(peak_headroom).min(MAX_GAIN_DB);

    let updated = upsert_output_gain_db(&raw, applied);
    fs::write(manifest_path, updated)
        .with_context(|| format!("write {}", manifest_path.display()))?;

    Ok(AuditReport {
        measured_lufs,
        measured_peak_dbfs,
        want_for_lufs_db: want_for_lufs,
        applied_gain_db: applied,
    })
}

/// Signed unity correction for one IR: `LUFS_in − LUFS_out`. Brings
/// the convolved output back to the DI's loudness. Negative for IRs
/// that add level, positive for those that lose it. Only the boost
/// side is bounded (true-peak guard + runaway cap); attenuation is
/// unbounded so a hot IR is fully tamed.
fn ir_capture_gain_db(di: &[f32], ir: &[f32]) -> f32 {
    let wet = convolve(di, ir);
    let lufs_in = integrated_lufs(di, DI_SAMPLE_RATE as u32);
    let lufs_out = integrated_lufs(&wet, DI_SAMPLE_RATE as u32);
    let peak_out = peak_dbfs(&wet);
    let want = lufs_in - lufs_out;
    let peak_headroom = PEAK_CEILING_DBFS - peak_out;
    want.min(peak_headroom).min(MAX_GAIN_DB)
}

fn audit_ir_plugin(
    plugin_dir: &Path,
    manifest_path: &Path,
    raw: &str,
    di: &[f32],
) -> Result<AuditReport> {
    let files = all_capture_files(raw);
    if files.is_empty() {
        bail!("no `captures:[].file` entry in manifest");
    }
    let mut gains: Vec<(String, f32)> = Vec::with_capacity(files.len());
    let mut sum = 0.0_f32;
    for f in &files {
        let ir = load_wav_ir(&plugin_dir.join(f))
            .with_context(|| format!("load IR {f}"))?;
        let g = ir_capture_gain_db(di, &ir);
        sum += g;
        gains.push((f.clone(), g));
    }
    let updated = upsert_capture_output_gain_db(raw, &gains);
    fs::write(manifest_path, updated)
        .with_context(|| format!("write {}", manifest_path.display()))?;
    let mean = sum / files.len() as f32;
    Ok(AuditReport {
        measured_lufs: f32::NAN,
        measured_peak_dbfs: f32::NAN,
        want_for_lufs_db: mean,
        applied_gain_db: mean,
    })
}

fn manifest_block_type(yaml: &str) -> Option<String> {
    for line in yaml.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("type:") {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

/// Blocks that take loudness normalisation. Each gets a calibrated
/// `output_gain_db` so toggling the block in the chain does NOT change
/// perceived volume — only tone/saturation.
///
/// `amp`/`preamp`/`gain_pedal` are NAM captures: the gain is measured
/// from the model output on the synthetic DI (issue #413).
///
/// `cab`/`body` are IR captures: an IR is a linear filter with real
/// insertion loss, so its makeup is measured per capture by convolving
/// the same synthetic DI through the `.wav` (issue #8). They are NOT
/// loudness-neutral spectral shapers — uncompensated they drop level.
fn is_loudness_normalisable(block_type: &str) -> bool {
    matches!(
        block_type,
        "amp" | "preamp" | "gain_pedal" | "cab" | "body"
    )
}

fn first_capture_file(yaml: &str) -> Option<String> {
    let mut in_captures = false;
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("captures:") {
            in_captures = true;
            continue;
        }
        if !in_captures {
            continue;
        }
        let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if let Some(rest) = after_dash.strip_prefix("file:") {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

/// Every `file:` under `captures:`, in document order.
fn all_capture_files(yaml: &str) -> Vec<String> {
    let mut in_captures = false;
    let mut files = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if !line.starts_with(char::is_whitespace) && trimmed.starts_with("captures:") {
            in_captures = true;
            continue;
        }
        if in_captures
            && !line.starts_with(char::is_whitespace)
            && !trimmed.starts_with('-')
            && !trimmed.is_empty()
        {
            break; // next column-0 mapping key ends the captures block
        }
        if !in_captures {
            continue;
        }
        let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if let Some(rest) = after_dash.strip_prefix("file:") {
            files.push(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    files
}

fn upsert_output_gain_db(yaml: &str, gain: f32) -> String {
    let new_line = format!("output_gain_db: {gain:.7}");
    let already_present = yaml
        .lines()
        .any(|l| l.trim_start().starts_with("output_gain_db:"));
    let trailing_newline = yaml.ends_with('\n');
    let body: String = if already_present {
        yaml.lines()
            .map(|l| {
                if l.trim_start().starts_with("output_gain_db:") {
                    new_line.clone()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        let mut out = Vec::with_capacity(yaml.lines().count() + 1);
        let mut inserted = false;
        for l in yaml.lines() {
            if !inserted && l.trim_start().starts_with("type:") {
                out.push(new_line.clone());
                inserted = true;
            }
            out.push(l.to_string());
        }
        if !inserted {
            out.push(new_line);
        }
        out.join("\n")
    };
    if trailing_newline {
        format!("{body}\n")
    } else {
        body
    }
}

/// Inserts/replaces `output_gain_db:` as a sibling of each capture's
/// `file:` line, at the same indentation, preserving all other YAML
/// bytes. Keyed by the `file:` value so order/structure is irrelevant.
fn upsert_capture_output_gain_db(yaml: &str, gains: &[(String, f32)]) -> String {
    let map: HashMap<&str, f32> =
        gains.iter().map(|(f, g)| (f.as_str(), *g)).collect();
    let trailing_newline = yaml.ends_with('\n');
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + gains.len());

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        out.push(line.to_string());

        if let Some(rest) = after_dash.strip_prefix("file:") {
            let file = rest.trim().trim_matches('"').trim_matches('\'');
            if let Some(&g) = map.get(file) {
                // Column of the `file` key = leading ws + an optional
                // "- " sequence prefix. Siblings sit at that column.
                let lead = line.len() - trimmed.len();
                let key_col = lead + if trimmed.starts_with("- ") { 2 } else { 0 };
                let indent = " ".repeat(key_col);
                let gain_line = format!("{indent}output_gain_db: {g:.7}");
                // Drop an existing sibling output_gain_db on the next line.
                if let Some(next) = lines.get(i + 1) {
                    if next.trim_start().starts_with("output_gain_db:")
                        && (next.len() - next.trim_start().len()) == key_col
                    {
                        i += 1; // skip stale line
                    }
                }
                out.push(gain_line);
            }
        }
        i += 1;
    }
    let body = out.join("\n");
    if trailing_newline {
        format!("{body}\n")
    } else {
        body
    }
}

// Silence dead-code warning when used only in the binary path.
#[allow(dead_code)]
fn _suppress_unused() {
    let _ = (db_to_lin(0.0), apply_output_limiter as fn(&mut [f32]));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_field_before_type() {
        let yaml = "id: nam_test\nbrand: foo\ntype: amp\nbackend: nam\n";
        let out = upsert_output_gain_db(yaml, 7.5);
        assert!(out.contains("output_gain_db: 7.5000000\ntype: amp"));
    }

    #[test]
    fn replaces_existing_field_in_place() {
        let yaml = "id: x\noutput_gain_db: 1.2\ntype: amp\n";
        let out = upsert_output_gain_db(yaml, 3.4);
        assert!(out.contains("output_gain_db: 3.4"));
        assert!(!out.contains("1.2"));
    }

    #[test]
    fn loudness_normalisable_set() {
        for ok in ["amp", "preamp", "gain_pedal", "cab", "body"] {
            assert!(is_loudness_normalisable(ok));
        }
        for skip in ["reverb", "delay", "filter", "utility"] {
            assert!(!is_loudness_normalisable(skip));
        }
    }

    #[test]
    fn finds_first_capture_file() {
        let yaml = "captures:\n- file: captures/clean.nam\n";
        assert_eq!(
            first_capture_file(yaml),
            Some("captures/clean.nam".to_string())
        );
    }

    #[test]
    fn reads_block_type() {
        let yaml = "id: x\ntype: amp\n";
        assert_eq!(manifest_block_type(yaml), Some("amp".to_string()));
    }

    #[test]
    fn lists_all_capture_files_in_order() {
        let yaml = "captures:\n- values:\n    mic: a\n  file: ir/one.wav\n- values:\n    mic: b\n  file: ir/two.wav\n";
        assert_eq!(
            all_capture_files(yaml),
            vec!["ir/one.wav".to_string(), "ir/two.wav".to_string()]
        );
    }

    #[test]
    fn inserts_gain_per_capture_after_file_line() {
        let yaml = "type: cab\ncaptures:\n- values:\n    mic: a\n  file: ir/one.wav\n- values:\n    mic: b\n  file: ir/two.wav\n";
        let gains = vec![
            ("ir/one.wav".to_string(), 4.0_f32),
            ("ir/two.wav".to_string(), 9.5_f32),
        ];
        let out = upsert_capture_output_gain_db(yaml, &gains);
        assert!(out.contains("  file: ir/one.wav\n  output_gain_db: 4.0000000"));
        assert!(out.contains("  file: ir/two.wav\n  output_gain_db: 9.5000000"));
        assert!(out.starts_with("type: cab\n"));
    }

    #[test]
    fn replaces_existing_per_capture_gain_in_place() {
        let yaml = "captures:\n- file: ir/one.wav\n  output_gain_db: 1.0000000\n";
        let gains = vec![("ir/one.wav".to_string(), 7.0_f32)];
        let out = upsert_capture_output_gain_db(yaml, &gains);
        assert!(out.contains("output_gain_db: 7.0000000"));
        assert!(!out.contains("1.0000000"));
    }

    #[test]
    fn unity_correction_is_signed() {
        let di = loudness_audit::synthetic_di::default_guitar_di();

        // ×0.5 IR (−6 dB): output is quieter -> POSITIVE makeup ≈ +6.
        let ir_atten = vec![0.5_f32];
        let g = ir_capture_gain_db(&di, &ir_atten);
        assert!(g > 4.0 && g < 8.0, "attenuating IR makeup was {g}");

        // ×2 IR (+6 dB): output is louder -> NEGATIVE correction,
        // the hot block is brought back down to unity (not 0).
        let ir_boost = vec![2.0_f32];
        let g2 = ir_capture_gain_db(&di, &ir_boost);
        assert!(g2 < -4.0 && g2 > -8.0, "amplifying IR must attenuate, was {g2}");
    }
}
