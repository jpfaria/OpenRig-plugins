//! `qa_audit` — automated quality gate for plugin outputs (issue #12).
//!
//! Runs every loudness-normalisable plugin (NAM `amp`/`preamp`/
//! `gain_pedal` and IR `cab`/`body`) through a deterministic probe
//! DI and asserts every threshold in [`loudness_audit::qa`]. Exit
//! non-zero on any failure; `pack_plugins` aborts the release on a
//! non-zero exit. Listening is not a valid verification step here.
//!
//! Usage:
//!
//!     cargo run --release -p loudness-audit --bin qa_audit -- \
//!         --source /path/to/OpenRig-plugins/plugins/source
//!
//! Also runs a per-chain check (default: `nam_ibanez_ts9` →
//! `nam_mesa_rectifier`) so the chained-gain failure mode that
//! gutted the cpm 22 preset is now caught automatically.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use loudness_audit::ir::{convolve, load_wav_ir};
use loudness_audit::loudness::{integrated_lufs, peak_dbfs};
use loudness_audit::qa::{
    check_clip, check_dc_offset, check_hf_aliasing, check_lufs_band, check_non_finite,
    check_silence, QaFail,
};
use loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};
use nam::processor::{close_model_diag, nam_process, open_model_diag};

/// Chain check lower bound: chain output must not collapse below this
/// integrated LUFS. -40 catches the "two amps in series each pulled to
/// unity → -13 dB summation" failure that surfaced on cpm 22.
const CHAIN_LUFS_FLOOR: f32 = -40.0;

fn main() {
    if let Err(e) = run() {
        eprintln!("qa_audit: {e:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let source = parse_source_arg(&args)?;
    if !source.is_dir() {
        bail!("--source not a directory: {}", source.display());
    }

    let di = default_guitar_di();
    let sr = DI_SAMPLE_RATE as u32;

    eprintln!(
        "qa_audit: probe = synthetic DI, {} samples @ {} Hz",
        di.len(),
        sr
    );
    eprintln!("source: {}", source.display());
    eprintln!();

    let mut fail_count = 0usize;
    let mut ok_count = 0usize;
    let mut skipped = 0usize;

    for kind in ["nam", "ir"] {
        let root = source.join(kind);
        if !root.is_dir() {
            continue;
        }
        let mut dirs: Vec<PathBuf> = fs::read_dir(&root)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for plugin_dir in dirs {
            let manifest = plugin_dir.join("manifest.yaml");
            if !manifest.is_file() {
                continue;
            }
            let raw = fs::read_to_string(&manifest)
                .with_context(|| format!("read {}", manifest.display()))?;
            let block_type = manifest_block_type(&raw).unwrap_or_else(|| "<?>".into());
            if !is_loudness_normalisable(&block_type) {
                skipped += 1;
                continue;
            }
            let label = plugin_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("<?>");

            let report: Result<Vec<(String, Vec<QaFail>)>> =
                if matches!(block_type.as_str(), "cab" | "body") {
                    audit_ir_plugin(&di, sr, &plugin_dir, &raw)
                } else {
                    audit_nam_plugin(&di, sr, &plugin_dir, &raw)
                        .map(|fails| vec![(String::from("<model>"), fails)])
                };

            match report {
                Ok(per) => {
                    let total_fails: usize = per.iter().map(|(_, f)| f.len()).sum();
                    if total_fails == 0 {
                        ok_count += 1;
                    } else {
                        fail_count += 1;
                        eprintln!("FAIL {kind}/{label} ({block_type})");
                        for (cap, fails) in &per {
                            for f in fails {
                                eprintln!("  - {cap}: {}: {:?}", f.label(), f);
                            }
                        }
                    }
                }
                Err(e) => {
                    fail_count += 1;
                    eprintln!("ERROR {kind}/{label}: {e:#}");
                }
            }
        }
    }

    // Chain checks (encode the cpm 22 failure mode).
    let chain_specs: &[&[&str]] = &[&["ibanez_ts9", "mesa_rectifier"]];
    eprintln!();
    eprintln!("-- chain checks --");
    for chain in chain_specs {
        let label = chain.join(" -> ");
        match audit_chain(&di, sr, &source.join("nam"), chain) {
            Ok(fails) => {
                if fails.is_empty() {
                    ok_count += 1;
                    eprintln!("ok   chain {label}");
                } else {
                    fail_count += 1;
                    eprintln!("FAIL chain {label}");
                    for f in &fails {
                        eprintln!("  - {}: {:?}", f.label(), f);
                    }
                }
            }
            Err(e) => {
                fail_count += 1;
                eprintln!("ERROR chain {label}: {e:#}");
            }
        }
    }

    eprintln!();
    eprintln!("qa_audit: ok={ok_count} fail={fail_count} skipped={skipped}");
    if fail_count > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn parse_source_arg(args: &[String]) -> Result<PathBuf> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--source" {
            let p = it
                .next()
                .ok_or_else(|| anyhow!("--source requires a path"))?;
            return Ok(PathBuf::from(p));
        }
    }
    bail!(
        "usage: qa_audit --source <plugins/source path>\n\
         exits non-zero if any plugin or chain check fails"
    )
}

// ---------------------------------------------------------------------------
// Orchestrators
// ---------------------------------------------------------------------------

fn check_all(probe: &[f32], out: &[f32], sr: u32) -> Vec<QaFail> {
    let mut fails = Vec::new();
    if let Some(f) = check_non_finite(out) {
        fails.push(f);
    }
    if let Some(f) = check_clip(out) {
        fails.push(f);
    }
    if let Some(f) = check_silence(out, sr) {
        fails.push(f);
    }
    if let Some(f) = check_lufs_band(out, sr) {
        fails.push(f);
    }
    if let Some(f) = check_dc_offset(out) {
        fails.push(f);
    }
    if let Some(f) = check_hf_aliasing(probe, out, sr) {
        fails.push(f);
    }
    fails
}

fn audit_nam_plugin(
    probe: &[f32],
    sr: u32,
    plugin_dir: &Path,
    raw: &str,
) -> Result<Vec<QaFail>> {
    let capture = first_capture_file(raw)
        .ok_or_else(|| anyhow!("no captures:[].file entry"))?;
    let path = plugin_dir.join(&capture);
    let out = run_nam(probe, &path)?;
    Ok(check_all(probe, &out, sr))
}

fn audit_ir_plugin(
    probe: &[f32],
    sr: u32,
    plugin_dir: &Path,
    raw: &str,
) -> Result<Vec<(String, Vec<QaFail>)>> {
    let files = all_capture_files(raw);
    if files.is_empty() {
        bail!("no captures:[].file entries");
    }
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let ir = load_wav_ir(&plugin_dir.join(&f))
            .with_context(|| format!("load IR {f}"))?;
        let wet = convolve(probe, &ir);
        out.push((f, check_all(probe, &wet, sr)));
    }
    Ok(out)
}

/// Runs the probe through each NAM in `chain` sequentially (output of
/// one feeds the next). Asserts final peak ≤ ceiling and final LUFS
/// ≥ `CHAIN_LUFS_FLOOR`. Failure modes encoded:
///   - additive chain-gain clip (boost-only hot-target era);
///   - chain collapse below floor (unity-LUFS per-block era).
fn audit_chain(
    probe: &[f32],
    sr: u32,
    nam_root: &Path,
    chain: &[&str],
) -> Result<Vec<QaFail>> {
    let mut signal = probe.to_vec();
    for plugin_name in chain {
        let plugin_dir = nam_root.join(plugin_name);
        let manifest = plugin_dir.join("manifest.yaml");
        if !manifest.is_file() {
            bail!("missing chain plugin: {}", plugin_dir.display());
        }
        let raw = fs::read_to_string(&manifest)?;
        let capture = first_capture_file(&raw)
            .ok_or_else(|| anyhow!("no captures in {plugin_name}"))?;
        signal = run_nam(&signal, &plugin_dir.join(&capture))
            .with_context(|| format!("run NAM {plugin_name}"))?;
    }
    let mut fails = Vec::new();
    if let Some(f) = check_non_finite(&signal) {
        fails.push(f);
    }
    if let Some(f) = check_clip(&signal) {
        fails.push(f);
    }
    let lufs = integrated_lufs(&signal, sr);
    if !lufs.is_finite() || lufs < CHAIN_LUFS_FLOOR {
        fails.push(QaFail::Silence { lufs });
    }
    // Diagnostic peak / lufs at the end of the chain.
    let peak = peak_dbfs(&signal);
    eprintln!(
        "     chain end: peak={peak:+.2} dBFS, lufs={lufs:+.2} LUFS"
    );
    Ok(fails)
}

fn run_nam(input: &[f32], model_path: &Path) -> Result<Vec<f32>> {
    let p = model_path
        .to_str()
        .ok_or_else(|| anyhow!("non-utf8 model path"))?;
    let model = open_model_diag(p).with_context(|| format!("open {p}"))?;
    let mut out = vec![0.0_f32; input.len()];
    unsafe {
        nam_process(model, input, &mut out);
        close_model_diag(model);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tiny manifest parsers — duplicated from `main.rs` to keep this binary
// self-contained. Behaviour-identical; covered by their tests there.
// ---------------------------------------------------------------------------

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
            break;
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
