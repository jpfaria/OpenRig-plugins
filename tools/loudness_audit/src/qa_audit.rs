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
//! Optional `--plugins kind/name[,kind/name…]` (issue #28) restricts
//! the audit to a subset for fast iteration on a single plugin without
//! reprocessing the whole tree:
//!
//!     cargo run --release -p loudness-audit --bin qa_audit -- \
//!         --source /path/to/OpenRig-plugins/plugins/source \
//!         --plugins nam/mesa_rectifier,ir/marshall_4x12
//!
//! Also runs a per-chain check (default: `nam_ibanez_ts9` →
//! `nam_mesa_rectifier`) so the chained-gain failure mode that
//! gutted the cpm 22 preset is now caught automatically. When a
//! `--plugins` filter excludes any chain member, that chain step is
//! skipped (not a failure) — the flag exists to focus on one plugin.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use loudness_audit::ir::{convolve, load_wav_ir};
use loudness_audit::loudness::{integrated_lufs, peak_dbfs};
use loudness_audit::qa::{
    check_clip, check_clip_with, check_dc_offset, check_dc_offset_with, check_hf_aliasing,
    check_hf_aliasing_with, check_lufs_band, check_lufs_band_with, check_non_finite,
    check_silence, check_silence_with, check_spectral_peak, CLIP_CEILING_NONLINEAR_DBFS,
    DC_THRESHOLD_NONLINEAR, HF_ALIASING_MARGIN_NONLINEAR_DB, LUFS_BAND_MAX,
    LUFS_BAND_MIN_NONLINEAR, QaFail, SILENCE_LUFS_BODY,
};
use loudness_audit::selector::PluginSelector;
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
    let selector = PluginSelector::from_args(&args)?;
    if let Some(s) = &selector {
        s.validate_against(&source)?;
    }

    let di = default_guitar_di();
    let sr = DI_SAMPLE_RATE as u32;

    eprintln!(
        "qa_audit: probe = synthetic DI, {} samples @ {} Hz",
        di.len(),
        sr
    );
    eprintln!("source: {}", source.display());
    if let Some(s) = &selector {
        let labels: Vec<String> = s
            .entries()
            .iter()
            .map(|(k, n)| format!("{k}/{n}"))
            .collect();
        eprintln!("filter: --plugins {} ({} entries)", labels.join(","), s.entries().len());
    }
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
            let label = plugin_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("<?>");
            // Filter-out is silent on purpose: the user opted-in to a
            // subset, the non-selected plugins are not part of this
            // run's universe so they don't count as skipped.
            if let Some(s) = &selector {
                if !s.matches(kind, label) {
                    continue;
                }
            }
            let raw = fs::read_to_string(&manifest)
                .with_context(|| format!("read {}", manifest.display()))?;
            let block_type = manifest_block_type(&raw).unwrap_or_else(|| "<?>".into());
            if !is_loudness_normalisable(&block_type) {
                skipped += 1;
                continue;
            }

            let report: Result<Vec<(String, Vec<QaFail>)>> =
                if matches!(block_type.as_str(), "cab" | "body") {
                    audit_ir_plugin(&di, sr, &plugin_dir, &raw, BlockClass::from_type(&block_type))
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

    // Chain checks (encode the cpm 22 failure mode). Chains are
    // NAM-only today. Under `--plugins`, a chain only runs if every
    // member is in the selection — otherwise the chain step is skipped
    // (not a failure): the flag exists for fast iteration on a single
    // plugin, and failing on chain summation the user didn't touch is
    // noise.
    let chain_specs: &[&[&str]] = &[&["ibanez_ts9", "mesa_rectifier"]];
    eprintln!();
    eprintln!("-- chain checks --");
    for chain in chain_specs {
        let label = chain.join(" -> ");
        if let Some(s) = &selector {
            let missing: Vec<&str> = chain
                .iter()
                .copied()
                .filter(|name| !s.matches("nam", name))
                .collect();
            if !missing.is_empty() {
                eprintln!("skip chain {label} (filtered: {})", missing.join(","));
                continue;
            }
        }
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
        "usage: qa_audit --source <plugins/source path> \
         [--plugins kind/name[,kind/name...]]\n\
         exits non-zero if any plugin or chain check fails"
    )
}

// ---------------------------------------------------------------------------
// Orchestrators
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
enum BlockClass {
    LinearCab,
    LinearBody,
    Nonlinear,
}

impl BlockClass {
    fn from_type(t: &str) -> Self {
        match t {
            "cab" => BlockClass::LinearCab,
            "body" => BlockClass::LinearBody,
            _ => BlockClass::Nonlinear, // amp / preamp / gain_pedal
        }
    }
}

fn check_all(probe: &[f32], out: &[f32], sr: u32, class: BlockClass) -> Vec<QaFail> {
    let mut fails = Vec::new();
    if let Some(f) = check_non_finite(out) {
        fails.push(f);
    }
    let clip = match class {
        BlockClass::LinearCab | BlockClass::LinearBody => check_clip(out),
        BlockClass::Nonlinear => check_clip_with(out, CLIP_CEILING_NONLINEAR_DBFS),
    };
    if let Some(f) = clip {
        fails.push(f);
    }
    let silence = match class {
        BlockClass::LinearBody => check_silence_with(out, sr, SILENCE_LUFS_BODY),
        _ => check_silence(out, sr),
    };
    if let Some(f) = silence {
        fails.push(f);
    }
    let lufs = match class {
        BlockClass::LinearCab => check_lufs_band(out, sr),
        BlockClass::LinearBody => {
            // Body IRs are pickup-emulation filters — narrow-band by
            // design. After the spectral-unity makeup (#23) brings
            // their max|H| to 0 dB, the convolved LUFS drops well
            // below any "sanity" floor without indicating a defect
            // (soundhole captures land at −55 to −67 LUFS). Dead
            // captures are still caught by `check_silence` at −60
            // applied unconditionally above; LUFS_BAND is skipped
            // for body.
            None
        }
        BlockClass::Nonlinear => {
            check_lufs_band_with(out, sr, LUFS_BAND_MIN_NONLINEAR, LUFS_BAND_MAX)
        }
    };
    if let Some(f) = lufs {
        fails.push(f);
    }
    let dc = match class {
        BlockClass::LinearCab | BlockClass::LinearBody => check_dc_offset(out),
        BlockClass::Nonlinear => check_dc_offset_with(out, DC_THRESHOLD_NONLINEAR),
    };
    if let Some(f) = dc {
        fails.push(f);
    }
    let hf = match class {
        BlockClass::LinearCab | BlockClass::LinearBody => check_hf_aliasing(probe, out, sr),
        BlockClass::Nonlinear => {
            check_hf_aliasing_with(probe, out, sr, HF_ALIASING_MARGIN_NONLINEAR_DB)
        }
    };
    if let Some(f) = hf {
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
    Ok(check_all(probe, &out, sr, BlockClass::Nonlinear))
}

fn audit_ir_plugin(
    probe: &[f32],
    sr: u32,
    plugin_dir: &Path,
    raw: &str,
    class: BlockClass,
) -> Result<Vec<(String, Vec<QaFail>)>> {
    let captures = all_captures_with_gain(raw);
    if captures.is_empty() {
        bail!("no captures:[].file entries");
    }
    let mut out = Vec::with_capacity(captures.len());
    for (f, gain_db) in captures {
        let ir = load_wav_ir(&plugin_dir.join(&f))
            .with_context(|| format!("load IR {f}"))?;
        // Apply manifest's per-capture output_gain_db to the raw IR
        // before any check — the engine convolves with this scaled
        // version, so the assertions must too (issue #23).
        let scale = 10f32.powf(gain_db / 20.0);
        let ir_scaled: Vec<f32> = ir.iter().map(|s| s * scale).collect();
        let wet = convolve(probe, &ir_scaled);
        let mut fails = check_all(probe, &wet, sr, class);
        // Spectral-peak check is IR-only (linear filter, no probe
        // needed), so it runs against the scaled IR directly.
        if let Some(f) = check_spectral_peak(&ir_scaled, sr) {
            fails.push(f);
        }
        out.push((f, fails));
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
    all_captures_with_gain(yaml)
        .into_iter()
        .map(|(f, _)| f)
        .collect()
}

/// Pairs each capture file with its sibling `output_gain_db` value
/// (defaulting to 0 if the field is absent — pre-#23 manifests). The
/// audit's spectral-peak check needs to apply the manifest gain to the
/// raw IR before measuring max|H| so the assertion mirrors what the
/// engine actually convolves with.
fn all_captures_with_gain(yaml: &str) -> Vec<(String, f32)> {
    let mut in_captures = false;
    let mut out: Vec<(String, f32)> = Vec::new();
    let mut current_file: Option<String> = None;
    let mut current_gain: f32 = 0.0;
    let flush = |out: &mut Vec<(String, f32)>, file: &mut Option<String>, gain: &mut f32| {
        if let Some(f) = file.take() {
            out.push((f, *gain));
            *gain = 0.0;
        }
    };
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
            flush(&mut out, &mut current_file, &mut current_gain);
            break;
        }
        if !in_captures {
            continue;
        }
        // A new list item closes the previous one.
        if trimmed.starts_with("- ") {
            flush(&mut out, &mut current_file, &mut current_gain);
        }
        let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if let Some(rest) = after_dash.strip_prefix("file:") {
            current_file = Some(
                rest.trim().trim_matches('"').trim_matches('\'').to_string(),
            );
        } else if let Some(rest) = after_dash.strip_prefix("output_gain_db:") {
            if let Ok(v) = rest.trim().parse::<f32>() {
                current_gain = v;
            }
        }
    }
    flush(&mut out, &mut current_file, &mut current_gain);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_capture_file_and_gain_pairs() {
        let yaml = "type: body\ncaptures:\n- values:\n    voicing: x\n  file: ir/a.wav\n  output_gain_db: -29.7897911\n- values:\n    voicing: y\n  file: ir/b.wav\n  output_gain_db: -24.5162811\n";
        let got = all_captures_with_gain(yaml);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].0, "ir/a.wav");
        assert!((got[0].1 - -29.7897911).abs() < 1e-3, "got {}", got[0].1);
        assert_eq!(got[1].0, "ir/b.wav");
        assert!((got[1].1 - -24.5162811).abs() < 1e-3, "got {}", got[1].1);
    }

    #[test]
    fn defaults_gain_to_zero_when_absent() {
        let yaml = "captures:\n- file: ir/a.wav\n";
        let got = all_captures_with_gain(yaml);
        assert_eq!(got, vec![("ir/a.wav".to_string(), 0.0)]);
    }
}
