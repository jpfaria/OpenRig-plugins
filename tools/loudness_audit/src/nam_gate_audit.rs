//! `nam_gate_audit` — measures the idle hiss of every NAM capture and
//! derives a per-capture noise-gate default deterministically (issue
//! #73, applied by OpenRig#675).
//!
//! ## Why
//!
//! High-gain NAM captures amplify the idle input noise floor (pickup /
//! cable / interface, "powered on, not playing"), so the block hisses
//! the moment it is enabled with no playing. OpenRig#675 lets a capture
//! ship its own `noise_gate` defaults in the manifest; this binary is
//! the measurement that decides, per capture, whether the gate is needed
//! and what input-referred `threshold_db` to ship.
//!
//! ## Method (measured, never guessed — CLAUDE.md "no validation by ear")
//!
//! 1. Feed a deterministic white-noise probe at `PROBE_RMS_DBFS`
//!    (−50 dBFS RMS, the idle floor jpfaria measured in OpenRig#675)
//!    through the raw `.nam` model; measure the output RMS.
//! 2. The audible idle hiss is the model output plus the manifest's
//!    `output_gain_db` (the engine applies it): `idle = out_rms + outg`.
//!    Small-signal *gain* alone is a poor discriminator — even clean
//!    amp channels amplify a −50 dBFS signal (a preamp has gain at tiny
//!    levels), so the catalogue gain is not bimodal. What the user hears
//!    is the *level*, so the gate decision keys on `idle`.
//! 3. A capture ships the gate on when `idle ≥ CUTOFF_AUDIBLE_IDLE_DBFS`
//!    (the clearly-audible tier; the cut is a reviewed policy choice, see
//!    issue #73). Quieter captures keep #612's gate-off default so their
//!    sustain is never strangled.
//! 4. The input-referred `threshold_db` is derived from the engine's own
//!    gate law (see `recommend_threshold_db`) to restore the idle floor.
//!
//! ## Modes
//!
//! - default (read-only): print a TSV report on stdout, one row per
//!   capture, plus a summary on stderr. Pipe to a file for `--apply`.
//! - `--apply <report.tsv>`: re-derive the decision from the report's
//!   measured columns (no re-measurement) and upsert the per-capture
//!   `noise_gate` block into each manifest. Idempotent: it strips any
//!   tool-written gate block first, so re-running with a different cutoff
//!   converges (clean captures lose a stale gate). This mirrors the
//!   tested `output_gain_db` writer in `main.rs` — never an ad-hoc
//!   transform script.
//!
//! Usage:
//!
//!     # measure
//!     cargo run --release -p loudness-audit --bin nam_gate_audit -- \
//!         --source plugins/source > gate_report.tsv
//!     # apply
//!     cargo run --release -p loudness-audit --bin nam_gate_audit -- \
//!         --source plugins/source --apply gate_report.tsv

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use loudness_audit::selector::PluginSelector;
use loudness_audit::synthetic_di::DI_SAMPLE_RATE;
use nam::processor::{close_model_diag, nam_process, open_model_diag};

/// Idle input noise floor used as the probe level. Matches the
/// OpenRig#675 root-cause measurement ("−50 dBFS RMS, simulating an idle
/// pickup/cable/interface"). RMS, not peak: the engine gate's detector
/// (`dsp::noise_gate::Trigger`) is a mean-square follower.
const PROBE_RMS_DBFS: f32 = -50.0;

/// Probe length. Two seconds of stationary noise is far more than the
/// gate's 10 ms detector time constant needs and gives a stable
/// output-RMS estimate. Deterministic (fixed seed) for CI repeatability.
const PROBE_SECONDS: f32 = 2.0;

fn main() {
    if let Err(e) = run() {
        eprintln!("nam_gate_audit: {e:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let source = parse_source_arg(&args)?;
    if !source.is_dir() {
        bail!("--source not a directory: {}", source.display());
    }
    if let Some(report) = parse_apply_arg(&args) {
        return apply(&source, &report);
    }
    measure(&source, PluginSelector::from_args(&args)?)
}

// ---------------------------------------------------------------------------
// Measurement (read-only report)
// ---------------------------------------------------------------------------

fn measure(source: &Path, selector: Option<PluginSelector>) -> Result<()> {
    if let Some(s) = &selector {
        s.validate_against(source)?;
    }
    let sr = DI_SAMPLE_RATE as u32;
    let probe = noise_probe(PROBE_SECONDS, PROBE_RMS_DBFS, sr);
    let probe_rms = rms_dbfs(&probe);
    eprintln!(
        "nam_gate_audit: probe = {} samples white noise @ {} Hz, RMS {:+.2} dBFS",
        probe.len(),
        sr,
        probe_rms
    );
    eprintln!("source: {}", source.display());
    eprintln!();

    println!("plugin\tcapture\tarch\tprobe_rms_dbfs\tout_rms_dbfs\tgain_db\toutput_gain_db\tidle_dbfs\trec_enabled\trec_threshold_db");

    let mut measured = 0usize;
    let mut errored = 0usize;
    let mut need_gate = 0usize;

    for plugin_dir in nam_plugin_dirs(source)? {
        let label = plugin_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<?>");
        if let Some(s) = &selector {
            if !s.matches("nam", label) {
                continue;
            }
        }
        let manifest = plugin_dir.join("manifest.yaml");
        let raw = fs::read_to_string(&manifest)
            .with_context(|| format!("read {}", manifest.display()))?;
        if !is_nam_gain_block(&manifest_block_type(&raw).unwrap_or_default()) {
            continue; // cab/body IR carry no model gain; gate is NAM-only
        }
        let arch = manifest_scalar(&raw, "architecture").unwrap_or_else(|| "?".into());
        let outg = manifest_output_gain_db(&raw);

        for capture in all_capture_files(&raw) {
            match measure_out_rms(&probe, &plugin_dir.join(&capture)) {
                Ok(out_rms) => {
                    let gain = out_rms - probe_rms;
                    let idle = out_rms + outg;
                    let thr = gate_threshold(idle);
                    let enabled = thr.is_some();
                    if enabled {
                        need_gate += 1;
                    }
                    println!(
                        "{label}\t{capture}\t{arch}\t{probe_rms:+.2}\t{out_rms:+.2}\t{gain:+.2}\t{outg:+.4}\t{idle:+.2}\t{enabled}\t{}",
                        thr.map(|t| format!("{t:+.1}")).unwrap_or_else(|| "-".into())
                    );
                    measured += 1;
                }
                Err(e) => {
                    eprintln!("ERROR {label}/{capture}: {e:#}");
                    errored += 1;
                }
            }
        }
    }

    eprintln!();
    eprintln!(
        "nam_gate_audit: measured {measured} captures, {need_gate} need gate (idle ≥ {CUTOFF_AUDIBLE_IDLE_DBFS:+.0} dBFS), {errored} errored"
    );
    if errored > 0 {
        std::process::exit(1);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Gate decision — the one place the policy lives
// ---------------------------------------------------------------------------

/// Audible idle-hiss level (model output + `output_gain_db`) at or above
/// which a capture ships the gate on. Reviewed policy choice (issue #73):
/// the clearly-audible tier. Quieter captures keep #612's gate-off
/// default to protect note sustain/decay.
const CUTOFF_AUDIBLE_IDLE_DBFS: f32 = -20.0;

/// Engine gate downward-expansion ratio
/// (`dsp::noise_gate::TriggerParams` in `cpp/nam_wrapper.cpp`). Below the
/// threshold the gate applies `reduction_dB = RATIO · (threshold − level)²`.
const GATE_RATIO: f32 = 0.1;

/// Where the gate pulls the idle hiss down to: the probe floor itself, so
/// the block no longer *amplifies* the idle noise (net unity at idle).
const TARGET_IDLE_DBFS: f32 = PROBE_RMS_DBFS;

/// Lower clamp — never ship a threshold at/below the idle floor (it would
/// be a no-op). Upper clamp — never gate above ordinary quiet playing.
const THRESHOLD_MIN_DBFS: f32 = -45.0;
const THRESHOLD_MAX_DBFS: f32 = -30.0;

/// Input-referred `threshold_db` for a capture whose measured audible
/// idle hiss is `idle_dbfs`, or `None` if the capture is below the cutoff
/// and ships no gate.
///
/// The gate sits before the model and the idle input sits at
/// `PROBE_RMS_DBFS`. To pull the idle hiss down to `TARGET_IDLE_DBFS` the
/// gate must apply `R = idle_dbfs − TARGET_IDLE_DBFS` dB of reduction at
/// the idle input level (the model and `output_gain_db` are ~linear at
/// the floor, so an input-referred reduction passes straight through).
/// Inverting the engine law `R = RATIO · (threshold − level)²` at
/// `level = PROBE_RMS_DBFS`:
///
/// ```text
/// threshold = PROBE_RMS_DBFS + sqrt(R / RATIO)
/// ```
///
/// Louder-idle captures therefore get a higher (less negative) threshold
/// — they gate harder — which matches both the physics and tight-gate
/// practice on high-gain amps. Clamped to a musically safe band.
fn gate_threshold(idle_dbfs: f32) -> Option<f32> {
    if idle_dbfs < CUTOFF_AUDIBLE_IDLE_DBFS {
        return None;
    }
    let reduction = idle_dbfs - TARGET_IDLE_DBFS;
    let thr = PROBE_RMS_DBFS + (reduction / GATE_RATIO).sqrt();
    Some(thr.clamp(THRESHOLD_MIN_DBFS, THRESHOLD_MAX_DBFS))
}

// ---------------------------------------------------------------------------
// Apply (writer) — consumes the measured report, upserts manifests
// ---------------------------------------------------------------------------

fn apply(source: &Path, report: &Path) -> Result<()> {
    let text = fs::read_to_string(report)
        .with_context(|| format!("read report {}", report.display()))?;
    // plugin -> { capture_file -> Some(threshold) | None }
    let mut by_plugin: BTreeMap<String, BTreeMap<String, Option<f32>>> = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue; // header / blank
        }
        let col: Vec<&str> = line.split('\t').collect();
        if col.len() < 8 {
            bail!("report line {} has {} columns, want ≥8", i + 1, col.len());
        }
        let plugin = col[0].to_string();
        let capture = col[1].to_string();
        let out_rms: f32 = col[4].parse().with_context(|| format!("out_rms line {}", i + 1))?;
        let outg: f32 = col[6].parse().with_context(|| format!("output_gain_db line {}", i + 1))?;
        let idle = out_rms + outg;
        by_plugin
            .entry(plugin)
            .or_default()
            .insert(capture, gate_threshold(idle));
    }

    let mut written = 0usize;
    let mut gated_captures = 0usize;
    for (plugin, decisions) in &by_plugin {
        let manifest = source.join("nam").join(plugin).join("manifest.yaml");
        if !manifest.is_file() {
            bail!("manifest not found for plugin {plugin}: {}", manifest.display());
        }
        let raw = fs::read_to_string(&manifest)?;
        let updated = upsert_capture_noise_gate(&raw, decisions);
        gated_captures += decisions.values().filter(|t| t.is_some()).count();
        if updated != raw {
            fs::write(&manifest, updated)
                .with_context(|| format!("write {}", manifest.display()))?;
            written += 1;
        }
    }
    eprintln!(
        "nam_gate_audit --apply: {} plugins updated, {} captures gated (idle ≥ {CUTOFF_AUDIBLE_IDLE_DBFS:+.0} dBFS)",
        written, gated_captures
    );
    Ok(())
}

/// Inserts/replaces a per-capture `noise_gate` block as a sibling of each
/// capture's `file:` line, keyed by the `file:` value. For a capture in
/// `decisions` with `Some(threshold)` the block is written; with `None`
/// any existing tool-written block is removed (idempotent re-apply). A
/// capture absent from `decisions` is left untouched. All other YAML
/// bytes are preserved.
fn upsert_capture_noise_gate(yaml: &str, decisions: &BTreeMap<String, Option<f32>>) -> String {
    let trailing_newline = yaml.ends_with('\n');
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + decisions.len() * 3);

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        out.push(line.to_string());

        if let Some(rest) = after_dash.strip_prefix("file:") {
            let file = rest.trim().trim_matches('"').trim_matches('\'');
            // Column of the `file` key = leading ws + an optional "- ".
            let lead = line.len() - trimmed.len();
            let key_col = lead + if trimmed.starts_with("- ") { 2 } else { 0 };

            // Drop an existing tool-written gate block on the following
            // lines (a `noise_gate:` sibling at key_col plus its more
            // deeply indented children). This keeps re-apply idempotent.
            if let Some(next) = lines.get(i + 1) {
                let nt = next.trim_start();
                let ncol = next.len() - nt.len();
                if ncol == key_col && nt.starts_with("noise_gate:") {
                    i += 1; // skip the `noise_gate:` line
                    while let Some(child) = lines.get(i + 1) {
                        let ct = child.trim_start();
                        let ccol = child.len() - ct.len();
                        if !ct.is_empty() && ccol > key_col {
                            i += 1; // skip an indented child
                        } else {
                            break;
                        }
                    }
                }
            }

            if let Some(Some(thr)) = decisions.get(file) {
                let indent = " ".repeat(key_col);
                out.push(format!("{indent}noise_gate:"));
                out.push(format!("{indent}  enabled: true"));
                out.push(format!("{indent}  threshold_db: {thr:.1}"));
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

// ---------------------------------------------------------------------------
// Model probe + DSP helpers
// ---------------------------------------------------------------------------

fn measure_out_rms(probe: &[f32], model_path: &Path) -> Result<f32> {
    let p = model_path
        .to_str()
        .ok_or_else(|| anyhow!("non-utf8 model path"))?;
    let model = open_model_diag(p).with_context(|| format!("open {p}"))?;
    let mut out = vec![0.0_f32; probe.len()];
    unsafe {
        nam_process(model, probe, &mut out);
        close_model_diag(model);
    }
    Ok(rms_dbfs(&out))
}

fn noise_probe(seconds: f32, target_rms_dbfs: f32, sr: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    let mut rng = XorShift64::new(0x6E0153_AA_5EED_u64);
    let mut buf: Vec<f32> = (0..n).map(|_| rng.next_f32_signed()).collect();
    let cur = rms_lin(&buf);
    if cur > 0.0 {
        let scale = 10f32.powf(target_rms_dbfs / 20.0) / cur;
        for s in buf.iter_mut() {
            *s *= scale;
        }
    }
    buf
}

fn rms_lin(buf: &[f32]) -> f32 {
    if buf.is_empty() {
        return 0.0;
    }
    (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
}

fn rms_dbfs(buf: &[f32]) -> f32 {
    let r = rms_lin(buf);
    if r <= 0.0 {
        -120.0
    } else {
        20.0 * r.log10()
    }
}

struct XorShift64 {
    state: u64,
}
impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xDEAD_BEEF } else { seed },
        }
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
    fn next_f32_signed(&mut self) -> f32 {
        (self.next_u64() as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0
    }
}

// ---------------------------------------------------------------------------
// Args + tiny manifest parsers (behaviour-identical to qa_audit's)
// ---------------------------------------------------------------------------

fn parse_source_arg(args: &[String]) -> Result<PathBuf> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--source" {
            let p = it.next().ok_or_else(|| anyhow!("--source requires a path"))?;
            return Ok(PathBuf::from(p));
        }
    }
    bail!(
        "usage: nam_gate_audit --source <plugins/source> [--plugins kind/name,...] [--apply report.tsv]"
    )
}

fn parse_apply_arg(args: &[String]) -> Option<PathBuf> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--apply" {
            return it.next().map(PathBuf::from);
        }
    }
    None
}

fn nam_plugin_dirs(source: &Path) -> Result<Vec<PathBuf>> {
    let root = source.join("nam");
    if !root.is_dir() {
        bail!("no nam/ under {}", source.display());
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("manifest.yaml").is_file())
        .collect();
    dirs.sort();
    Ok(dirs)
}

fn manifest_block_type(yaml: &str) -> Option<String> {
    manifest_scalar(yaml, "type")
}

fn manifest_scalar(yaml: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for line in yaml.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(&prefix) {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn manifest_output_gain_db(yaml: &str) -> f32 {
    for line in yaml.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("output_gain_db:") {
            return rest.trim().parse().unwrap_or(0.0);
        }
    }
    0.0
}

fn is_nam_gain_block(block_type: &str) -> bool {
    matches!(block_type, "amp" | "preamp" | "gain_pedal")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_hits_target_rms() {
        let p = noise_probe(0.5, -50.0, 48_000);
        assert!((rms_dbfs(&p) - -50.0).abs() < 0.1, "rms was {}", rms_dbfs(&p));
    }

    #[test]
    fn probe_is_deterministic() {
        assert_eq!(noise_probe(0.1, -50.0, 48_000), noise_probe(0.1, -50.0, 48_000));
    }

    #[test]
    fn quiet_capture_ships_no_gate() {
        assert_eq!(gate_threshold(CUTOFF_AUDIBLE_IDLE_DBFS - 0.1), None);
    }

    #[test]
    fn loud_idle_gets_a_threshold_in_band() {
        let t = gate_threshold(-17.7).expect("loud idle gates");
        assert!((THRESHOLD_MIN_DBFS..=THRESHOLD_MAX_DBFS).contains(&t), "thr {t}");
    }

    #[test]
    fn threshold_rises_with_idle_level() {
        let lo = gate_threshold(-20.0).unwrap();
        let hi = gate_threshold(-12.0).unwrap();
        assert!(hi > lo, "expected {hi} > {lo}");
    }

    #[test]
    fn threshold_inverts_the_engine_gate_law() {
        // Inside the clamp band the recommended threshold must reproduce
        // the target reduction through the engine's own law.
        let idle = -19.0_f32;
        let thr = gate_threshold(idle).unwrap();
        if (THRESHOLD_MIN_DBFS..THRESHOLD_MAX_DBFS).contains(&thr) {
            let r = GATE_RATIO * (thr - PROBE_RMS_DBFS).powi(2);
            assert!((r - (idle - TARGET_IDLE_DBFS)).abs() < 0.5, "law mismatch");
        }
    }

    #[test]
    fn upsert_inserts_block_under_gated_capture_only() {
        let yaml = "type: amp\ncaptures:\n- values:\n    voicing: dirty\n  file: captures/a.nam\n- values:\n    voicing: clean\n  file: captures/b.nam\n";
        let mut d = BTreeMap::new();
        d.insert("captures/a.nam".to_string(), Some(-32.0_f32));
        d.insert("captures/b.nam".to_string(), None);
        let out = upsert_capture_noise_gate(yaml, &d);
        assert!(out.contains("  file: captures/a.nam\n  noise_gate:\n    enabled: true\n    threshold_db: -32.0"));
        // clean capture untouched
        assert!(out.contains("  file: captures/b.nam\n"));
        assert_eq!(out.matches("noise_gate:").count(), 1);
    }

    #[test]
    fn upsert_is_idempotent_and_updates_threshold() {
        let yaml = "type: amp\ncaptures:\n- values:\n    voicing: dirty\n  file: captures/a.nam\n";
        let mut d = BTreeMap::new();
        d.insert("captures/a.nam".to_string(), Some(-32.0_f32));
        let once = upsert_capture_noise_gate(yaml, &d);
        // re-apply with a new threshold replaces, does not stack
        d.insert("captures/a.nam".to_string(), Some(-31.0_f32));
        let twice = upsert_capture_noise_gate(&once, &d);
        assert_eq!(twice.matches("noise_gate:").count(), 1);
        assert!(twice.contains("threshold_db: -31.0"));
        assert!(!twice.contains("-32.0"));
    }

    #[test]
    fn upsert_removes_stale_gate_when_capture_drops_below_cutoff() {
        let gated = "type: amp\ncaptures:\n- file: captures/a.nam\n  noise_gate:\n    enabled: true\n    threshold_db: -32.0\n";
        let mut d = BTreeMap::new();
        d.insert("captures/a.nam".to_string(), None);
        let out = upsert_capture_noise_gate(gated, &d);
        assert!(!out.contains("noise_gate:"), "stale gate not removed:\n{out}");
        assert!(out.contains("- file: captures/a.nam"));
    }

    #[test]
    fn lists_all_capture_files_in_order() {
        let yaml = "type: amp\ncaptures:\n- values:\n    voicing: a\n  file: captures/one.nam\n- values:\n    voicing: b\n  file: captures/two.nam\n";
        assert_eq!(
            all_capture_files(yaml),
            vec!["captures/one.nam".to_string(), "captures/two.nam".to_string()]
        );
    }

    #[test]
    fn reads_arch_and_type_scalars() {
        let yaml = "id: x\narchitecture: A2\ntype: amp\n";
        assert_eq!(manifest_scalar(yaml, "architecture"), Some("A2".into()));
        assert_eq!(manifest_block_type(yaml), Some("amp".into()));
    }
}
