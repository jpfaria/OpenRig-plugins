//! `loudness_audit` — writes `output_gain_db` into each plugin
//! `manifest.yaml` so every block plays at the maximum level that does
//! not clip (issue #143). The policy lives in [`loudness_audit::level`];
//! this binary measures and writes:
//!
//! - NAM (`amp`/`preamp`/`gain_pedal`): the synthetic DI is run through
//!   EVERY capture. One manifest-level value — the engine seeds one per
//!   block — puts the loudest capture's peak on the target.
//! - IR (`cab`/`body`): one value PER capture (issue #8; the engine
//!   re-seeds the Output knob when the capture changes). A cab is levelled
//!   against amp-level probes, a body against the DI.
//!
//! `output_gain_db` is the user-visible DEFAULT of the block's Output
//! knob, seeded by the engine's block factory — not an invisible internal
//! correction. It is signed (quiet blocks boosted, hot ones cut) and
//! clamped to the knob range. Supersedes the boost-only LUFS rule (#4) and
//! the spectral-unity IR rule (#23).
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

use loudness_audit::ir::load_wav_ir;
use loudness_audit::level::{ir_level_peak_dbfs, target_gain_db, IrRole, TARGET_PEAK_DBFS};
use loudness_audit::nam_run::nam_level_peaks_dbfs;
use loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

/// NAM `output_gain_db`: the engine applies one value to every capture of
/// the block, so it is sized on the loudest one — no capture may clip.
fn nam_gain_db(capture_peaks_dbfs: &[f32]) -> f32 {
    target_gain_db(
        capture_peaks_dbfs
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max),
    )
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: loudness_audit <plugins-root>");
        eprintln!();
        eprintln!("Expects a directory whose immediate children are NAM or IR");
        eprintln!("plugin packages (each carrying its own manifest.yaml).");
        std::process::exit(2);
    }
    let root = PathBuf::from(&args[1]);
    if !root.is_dir() {
        bail!("not a directory: {}", root.display());
    }

    let di = default_guitar_di();

    eprintln!("DI: {} samples @ {} Hz", di.len(), DI_SAMPLE_RATE as u32);
    eprintln!("target peak {TARGET_PEAK_DBFS:+.2} dBFS (max level without clipping)");
    eprintln!();
    eprintln!(
        "{:<48} {:>5} {:>8} {:>8}",
        "plugin", "caps", "peak", "applied"
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
        let label = plugin_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<?>")
            .to_string();
        match audit_plugin(&plugin_dir, &manifest_path, &di) {
            Ok(report) => {
                eprintln!(
                    "{:<48} {:>5} {:>+7.2}  {:>+7.2}",
                    label, report.captures, report.peak_dbfs, report.applied_gain_db
                );
                audited += 1;
            }
            Err(e) => {
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
    captures: usize,
    /// Loudest pre-gain reference peak across the captures (dBFS).
    peak_dbfs: f32,
    /// The NAM value, or the mean of the per-capture IR values.
    applied_gain_db: f32,
}

fn audit_plugin(plugin_dir: &Path, manifest_path: &Path, di: &[f32]) -> Result<AuditReport> {
    let raw = fs::read_to_string(manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let block_type = manifest_block_type(&raw).unwrap_or_else(|| "<unknown>".into());
    if !is_loudness_normalisable(&block_type) {
        bail!("type `{block_type}` is not loudness-normalised");
    }
    match block_type.as_str() {
        "cab" => return audit_ir_plugin(plugin_dir, manifest_path, &raw, di, IrRole::Cab),
        "body" => return audit_ir_plugin(plugin_dir, manifest_path, &raw, di, IrRole::Body),
        _ => {}
    }
    let models: Vec<PathBuf> = all_capture_files(&raw)
        .into_iter()
        .map(|f| plugin_dir.join(f))
        .collect();
    if models.is_empty() {
        bail!("no `captures:[].file` entry in manifest");
    }
    let peaks = nam_level_peaks_dbfs(di, &models)?;
    let applied = nam_gain_db(&peaks);

    let updated = upsert_output_gain_db(&raw, applied);
    fs::write(manifest_path, updated)
        .with_context(|| format!("write {}", manifest_path.display()))?;

    Ok(AuditReport {
        captures: models.len(),
        peak_dbfs: peaks.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        applied_gain_db: applied,
    })
}

fn audit_ir_plugin(
    plugin_dir: &Path,
    manifest_path: &Path,
    raw: &str,
    di: &[f32],
    role: IrRole,
) -> Result<AuditReport> {
    let files = all_capture_files(raw);
    if files.is_empty() {
        bail!("no `captures:[].file` entry in manifest");
    }
    let mut gains: Vec<(String, f32)> = Vec::with_capacity(files.len());
    let mut loudest = f32::NEG_INFINITY;
    for f in &files {
        let ir = load_wav_ir(&plugin_dir.join(f)).with_context(|| format!("load IR {f}"))?;
        let peak = ir_level_peak_dbfs(&ir, di, role);
        if !peak.is_finite() {
            return Err(anyhow!("IR {f} has no measurable output"));
        }
        loudest = loudest.max(peak);
        gains.push((f.clone(), target_gain_db(peak)));
    }
    let updated = upsert_capture_output_gain_db(raw, &gains);
    fs::write(manifest_path, updated)
        .with_context(|| format!("write {}", manifest_path.display()))?;
    let mean = gains.iter().map(|(_, g)| g).sum::<f32>() / gains.len() as f32;
    Ok(AuditReport {
        captures: files.len(),
        peak_dbfs: loudest,
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

/// Blocks that get a calibrated `output_gain_db`: NAM `amp`/`preamp`/
/// `gain_pedal` (measured through the model) and IR `cab`/`body`
/// (measured by convolution, per capture).
fn is_loudness_normalisable(block_type: &str) -> bool {
    matches!(
        block_type,
        "amp" | "preamp" | "gain_pedal" | "cab" | "body"
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nam_gain_sizes_on_loudest_capture() {
        // clean -20, lead -4 dBFS: one value for the whole block, so the
        // lead capture must land on the target and never clip.
        let g = nam_gain_db(&[-20.0, -4.0, -9.0]);
        assert!((g - (loudness_audit::level::TARGET_PEAK_DBFS + 4.0)).abs() < 1e-4);
    }

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
    fn capture_list_stops_at_next_top_level_key() {
        let yaml = "captures:\n- file: captures/a.nam\n  noise_gate:\n    enabled: true\nnoise_gate:\n  enabled: false\n";
        assert_eq!(all_capture_files(yaml), vec!["captures/a.nam".to_string()]);
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
}
