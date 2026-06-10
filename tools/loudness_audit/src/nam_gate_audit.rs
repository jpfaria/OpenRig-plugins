//! `nam_gate_audit` — derive each NAM capture's noise-gate `threshold_db`
//! by simulating the ENGINE'S ACTUAL gate in closed loop, per capture
//! (issue #73 follow-up). NO uniform formula: every value is measured
//! against that specific model.
//!
//! ## Why a closed loop (and not a formula)
//!
//! The first pass shipped a near-constant threshold derived by inverting
//! the gate's static law. Measuring the REAL gate (`dsp::noise_gate`,
//! with its 10 ms follower + open/hold/close state machine) on fluctuating
//! noise shows a completely different, per-model curve: the gate barely
//! engages until the threshold is within ~15 dB of the idle level, so the
//! formula's value did almost nothing for the hottest captures. The only
//! honest way to set the threshold is to run `gate -> model` and measure.
//!
//! ## Method (per capture, measured)
//!
//! Open the model once. Then:
//! 1. `idle_off` = RMS of the model on a -50 dBFS noise probe. A capture
//!    ships the gate on when `idle_off >= CUTOFF_AUDIBLE_IDLE_DBFS`.
//! 2. Binary-search `t_idle` = the gentlest threshold whose gated idle
//!    output falls to `IDLE_TARGET_DBFS` (run `gate(noise,T) -> model`).
//! 3. Binary-search `t_sustain` = the hardest threshold whose gated DI
//!    keeps its loudness within `SUSTAIN_TOL_DB` (run `gate(DI,T) -> model`,
//!    compare integrated LUFS). The measured "below playing" ceiling.
//! 4. `threshold = min(t_idle, t_sustain)`: silence idle as much as the
//!    capture's own sustain allows. Reported with the validation numbers
//!    (idle before/after, sustain loss) so each plugin is auditable.
//!
//! ## Modes
//! - default: TSV report (one row per capture) + summary.
//! - `--apply <report.tsv>`: transcribe the measured `threshold_db` into
//!   each manifest's per-capture `noise_gate` block (idempotent).
//! - `--probe <model.nam>`: print the full per-T curve for ONE capture
//!   (the per-plugin validator).
//!
//! Usage:
//!     nam_gate_audit --source plugins/source > report.tsv
//!     nam_gate_audit --source plugins/source --apply report.tsv
//!     nam_gate_audit --probe plugins/source/nam/<p>/captures/<c>.nam

use anyhow::{anyhow, bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use loudness_audit::loudness::integrated_lufs;
use loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};
use nam::processor::{close_model_diag, nam_process, open_model_diag};

const PROBE_RMS_DBFS: f32 = -50.0;
const SR_F: f64 = 48_000.0;

/// Audible idle-hiss level at/above which a capture ships the gate on.
const CUTOFF_AUDIBLE_IDLE_DBFS: f32 = -20.0;
/// Where gating should pull the idle hiss down to — clearly quiet
/// (~18 dB under a +32 dB capture's idle) without demanding the most
/// aggressive gate, which would also clamp soft playing.
const IDLE_TARGET_DBFS: f32 = -35.0;
/// Max integrated-LUFS loss the gate may cost the DI (the "below playing"
/// budget). Measured per capture, not a guessed threshold clamp.
const SUSTAIN_TOL_DB: f32 = 0.5;
/// Threshold search bounds (input-referred dBFS). Below the lower bound a
/// gate is a no-op on a -50 idle; the upper bound caps aggressiveness so
/// the gate stays below ordinary playing (the loud DI cannot probe soft
/// single-note playing, so this ceiling is the real protection there).
const T_LO: f32 = -45.0;
const T_HI: f32 = -30.0;

// ---------------------------------------------------------------------------
// Ported engine gate — dsp::noise_gate::Trigger + Gain (AudioDSPTools),
// with cpp/nam_wrapper.cpp params. Byte-faithful to the engine.
// ---------------------------------------------------------------------------

const GATE_TIME: f64 = 0.01;
const GATE_RATIO: f64 = 0.1;
const GATE_OPEN_TIME: f64 = 0.005;
const GATE_HOLD_TIME: f64 = 0.01;
const GATE_CLOSE_TIME: f64 = 0.05;
const MIN_LOUDNESS_DB: f64 = -120.0;

fn gain_reduction_db(level_db: f64, threshold: f64) -> f64 {
    if level_db < threshold {
        -GATE_RATIO * (level_db - threshold) * (level_db - threshold)
    } else {
        0.0
    }
}

/// Apply the engine gate to `input` at `threshold` (input-referred dBFS),
/// returning the gated signal (the model's input in the real chain).
fn gate(input: &[f32], threshold: f64) -> Vec<f32> {
    let alpha = 0.5f64.powf(1.0 / (GATE_TIME * SR_F));
    let beta = 1.0 - alpha;
    let dt = 1.0 / SR_F;
    let max_gr = gain_reduction_db(MIN_LOUDNESS_DB, threshold);
    let d_open = -max_gr / GATE_OPEN_TIME * dt;
    let d_close = max_gr / GATE_CLOSE_TIME * dt;
    let mlp = 10f64.powf(MIN_LOUDNESS_DB / 10.0);

    let mut holding = true;
    let mut level = 0.0f64;
    let mut last_gr = 0.0f64;
    let mut time_held = 0.0f64;

    let mut out = vec![0.0f32; input.len()];
    for (i, &x) in input.iter().enumerate() {
        let xd = x as f64;
        level = (alpha * level + beta * (xd * xd)).clamp(mlp, 1000.0);
        let level_db = 10.0 * level.log10();
        let gr = if holding {
            if level_db < threshold {
                time_held += dt;
                if time_held >= GATE_HOLD_TIME {
                    holding = false;
                }
            } else {
                time_held = 0.0;
            }
            last_gr = 0.0;
            0.0
        } else {
            let target = gain_reduction_db(level_db, threshold);
            if target > last_gr {
                last_gr += (0.5 * (target - last_gr)).clamp(0.0, d_open);
                if last_gr >= 0.0 {
                    last_gr = 0.0;
                    holding = true;
                    time_held = 0.0;
                }
            } else if target < last_gr {
                last_gr += (0.5 * (target - last_gr)).clamp(d_close, 0.0);
                if last_gr < max_gr {
                    last_gr = max_gr;
                }
            }
            last_gr
        };
        out[i] = (xd * 10f64.powf(gr / 20.0)) as f32;
    }
    out
}

// ---------------------------------------------------------------------------
// Model handle held open across a capture's whole T sweep.
// ---------------------------------------------------------------------------

struct Model(*mut std::ffi::c_void);
impl Model {
    fn open(path: &Path) -> Result<Self> {
        let p = path.to_str().ok_or_else(|| anyhow!("non-utf8 path"))?;
        let h = open_model_diag(p).with_context(|| format!("open {p}"))?;
        Ok(Model(h))
    }
    fn run(&self, input: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0f32; input.len()];
        unsafe { nam_process(self.0, input, &mut out) };
        out
    }
}
impl Drop for Model {
    fn drop(&mut self) {
        unsafe { close_model_diag(self.0) };
    }
}

fn rms_dbfs(b: &[f32]) -> f32 {
    if b.is_empty() {
        return -120.0;
    }
    let r = (b.iter().map(|s| s * s).sum::<f32>() / b.len() as f32).sqrt();
    if r <= 0.0 {
        -120.0
    } else {
        20.0 * r.log10()
    }
}

fn noise_probe(seconds: f32, target_rms_dbfs: f32, sr: u32) -> Vec<f32> {
    let n = (seconds * sr as f32) as usize;
    let mut s = 0x6E0153_AA_5EEDu64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0
    };
    let mut b: Vec<f32> = (0..n).map(|_| next()).collect();
    let cur = (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
    let scale = 10f32.powf(target_rms_dbfs / 20.0) / cur;
    for x in b.iter_mut() {
        *x *= scale;
    }
    b
}

// ---------------------------------------------------------------------------
// Per-capture closed-loop derivation
// ---------------------------------------------------------------------------

struct Derived {
    idle_off: f32,
    enabled: bool,
    threshold: f32,
    idle_after: f32,
    idle_suppress: f32,
    sustain_loss: f32,
}

/// Binary-search a monotonic threshold predicate over `[T_LO, T_HI]`.
/// `smallest_true_rising = true`  -> smallest T where `pred` becomes true
/// (pred false-low, true-high). `false` -> largest T where `pred` holds
/// (pred true-low, false-high).
fn search<F: FnMut(f32) -> bool>(mut pred: F, smallest_true_rising: bool) -> f32 {
    let (mut lo, mut hi) = (T_LO, T_HI);
    for _ in 0..7 {
        let mid = 0.5 * (lo + hi);
        let ok = pred(mid);
        if smallest_true_rising == ok {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

fn derive(model: &Model, noise: &[f32], di: &[f32], di_lufs_off: f32) -> Derived {
    let idle_off = rms_dbfs(&model.run(noise));
    if idle_off < CUTOFF_AUDIBLE_IDLE_DBFS {
        return Derived {
            idle_off,
            enabled: false,
            threshold: 0.0,
            idle_after: idle_off,
            idle_suppress: 0.0,
            sustain_loss: 0.0,
        };
    }
    let sr = DI_SAMPLE_RATE as u32;
    let sustain_loss_at = |t: f32| di_lufs_off - integrated_lufs(&model.run(&gate(di, t as f64)), sr);
    // t_idle: idle_out falls as T rises -> smallest T making (idle_out <= target).
    let t_idle = search(
        |t| rms_dbfs(&model.run(&gate(noise, t as f64))) <= IDLE_TARGET_DBFS,
        true,
    );
    // Gated captures are high-gain (loud DI), so the gate almost never bites
    // the DI within the search band. Check sustain at t_idle first; only run
    // the full t_sustain search (the expensive part) if it actually binds.
    let threshold = if sustain_loss_at(t_idle) <= SUSTAIN_TOL_DB {
        t_idle
    } else {
        // t_sustain: loss rises as T rises -> largest T keeping (loss <= tol).
        let t_sus = search(|t| sustain_loss_at(t) <= SUSTAIN_TOL_DB, false);
        t_idle.min(t_sus)
    };
    let threshold = ((threshold.clamp(T_LO, T_HI)) * 2.0).round() / 2.0; // 0.5 dB
    let idle_after = rms_dbfs(&model.run(&gate(noise, threshold as f64)));
    let sus_lufs = integrated_lufs(&model.run(&gate(di, threshold as f64)), sr);
    Derived {
        idle_off,
        enabled: true,
        threshold,
        idle_after,
        idle_suppress: idle_off - idle_after,
        sustain_loss: di_lufs_off - sus_lufs,
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

fn main() {
    if let Err(e) = run() {
        eprintln!("nam_gate_audit: {e:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(p) = flag(&args, "--probe") {
        return probe_one(Path::new(&p));
    }
    let source = flag(&args, "--source").map(PathBuf::from).ok_or_else(|| {
        anyhow!("usage: nam_gate_audit --source <plugins/source> [--apply report.tsv] [--probe model.nam]")
    })?;
    if let Some(report) = flag(&args, "--apply") {
        return apply(&source, Path::new(&report));
    }
    measure(&source, flag(&args, "--plugins"))
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Single-capture validator: print the full per-T curve.
fn probe_one(model_path: &Path) -> Result<()> {
    let sr = DI_SAMPLE_RATE as u32;
    let noise = noise_probe(2.0, PROBE_RMS_DBFS, sr);
    let di = default_guitar_di();
    let m = Model::open(model_path)?;
    let idle_off = rms_dbfs(&m.run(&noise));
    let di_lufs_off = integrated_lufs(&m.run(&di), sr);
    println!("# {}", model_path.display());
    println!(
        "# idle_off={idle_off:+.2} dBFS (gain {:+.2}) di_lufs_off={di_lufs_off:+.2}",
        idle_off - PROBE_RMS_DBFS
    );
    println!("T\tidle_out\tidle_suppress\tsustain_loss");
    let mut t = T_LO;
    while t <= T_HI {
        let io = rms_dbfs(&m.run(&gate(&noise, t as f64)));
        let sl = di_lufs_off - integrated_lufs(&m.run(&gate(&di, t as f64)), sr);
        println!("{t:+.1}\t{io:+.2}\t{:+.2}\t{sl:+.2}", idle_off - io);
        t += 1.0;
    }
    Ok(())
}

fn measure(source: &Path, plugins: Option<String>) -> Result<()> {
    let sr = DI_SAMPLE_RATE as u32;
    // 1.5 s of stationary noise is plenty for a stable idle-RMS estimate
    // (the gate's follower settles in ~60 ms) and keeps the sweep affordable.
    let noise = noise_probe(1.5, PROBE_RMS_DBFS, sr);
    // One power chord (attack + full natural decay) is enough to score the
    // sustain budget and keeps the per-capture sweep affordable.
    let di_full = default_guitar_di();
    let di: Vec<f32> = di_full[..(4.0 * sr as f32) as usize].to_vec();

    let only: Option<Vec<String>> =
        plugins.map(|s| s.split(',').map(|x| x.trim().to_string()).collect());

    eprintln!(
        "nam_gate_audit: closed-loop gate derivation (idle target {IDLE_TARGET_DBFS:+.0} dBFS, sustain budget {SUSTAIN_TOL_DB} dB)"
    );
    println!("plugin\tcapture\tidle_off_dbfs\tenabled\tthreshold_db\tidle_after_dbfs\tidle_suppress_db\tsustain_loss_db");

    let root = source.join("nam");
    let mut dirs: Vec<PathBuf> = fs::read_dir(&root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("manifest.yaml").is_file())
        .collect();
    dirs.sort();

    let (mut measured, mut gated, mut errored) = (0usize, 0usize, 0usize);
    for dir in dirs {
        let label = dir.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        if let Some(o) = &only {
            if !o.iter().any(|x| x == label) {
                continue;
            }
        }
        let raw = fs::read_to_string(dir.join("manifest.yaml"))?;
        if !is_nam_gain_block(&manifest_scalar(&raw, "type").unwrap_or_default()) {
            continue;
        }
        for cap in all_capture_files(&raw) {
            let path = dir.join(&cap);
            match Model::open(&path) {
                Ok(m) => {
                    let di_lufs_off = integrated_lufs(&m.run(&di), sr);
                    let d = derive(&m, &noise, &di, di_lufs_off);
                    println!(
                        "{label}\t{cap}\t{:+.2}\t{}\t{}\t{:+.2}\t{:+.2}\t{:+.2}",
                        d.idle_off,
                        d.enabled,
                        if d.enabled {
                            format!("{:+.1}", d.threshold)
                        } else {
                            "-".into()
                        },
                        d.idle_after,
                        d.idle_suppress,
                        d.sustain_loss
                    );
                    measured += 1;
                    if d.enabled {
                        gated += 1;
                    }
                }
                Err(e) => {
                    eprintln!("ERROR {label}/{cap}: {e:#}");
                    errored += 1;
                }
            }
        }
    }
    eprintln!("nam_gate_audit: measured {measured}, gated {gated}, errored {errored}");
    if errored > 0 {
        std::process::exit(1);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Apply: transcribe the measured threshold into manifests
// ---------------------------------------------------------------------------

fn apply(source: &Path, report: &Path) -> Result<()> {
    let text = fs::read_to_string(report)?;
    let mut by_plugin: BTreeMap<String, BTreeMap<String, Option<f32>>> = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 || line.trim().is_empty() {
            continue;
        }
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() < 5 {
            bail!("report line {} has {} cols", i + 1, c.len());
        }
        let enabled = c[3] == "true";
        let thr = if enabled {
            Some(c[4].parse::<f32>().with_context(|| format!("threshold line {}", i + 1))?)
        } else {
            None
        };
        by_plugin.entry(c[0].into()).or_default().insert(c[1].into(), thr);
    }
    let (mut written, mut gated) = (0usize, 0usize);
    for (plugin, decisions) in &by_plugin {
        let manifest = source.join("nam").join(plugin).join("manifest.yaml");
        let raw = fs::read_to_string(&manifest).with_context(|| format!("read {plugin}"))?;
        let updated = upsert_capture_noise_gate(&raw, decisions);
        gated += decisions.values().filter(|t| t.is_some()).count();
        if updated != raw {
            fs::write(&manifest, updated)?;
            written += 1;
        }
    }
    eprintln!("nam_gate_audit --apply: {written} plugins updated, {gated} captures gated");
    Ok(())
}

/// Insert/replace a per-capture `noise_gate` block keyed by `file:`.
fn upsert_capture_noise_gate(yaml: &str, decisions: &BTreeMap<String, Option<f32>>) -> String {
    let trailing = yaml.ends_with('\n');
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
            let lead = line.len() - trimmed.len();
            let key_col = lead + if trimmed.starts_with("- ") { 2 } else { 0 };
            if let Some(next) = lines.get(i + 1) {
                let nt = next.trim_start();
                if (next.len() - nt.len()) == key_col && nt.starts_with("noise_gate:") {
                    i += 1;
                    while let Some(ch) = lines.get(i + 1) {
                        let ct = ch.trim_start();
                        if !ct.is_empty() && (ch.len() - ct.len()) > key_col {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            if let Some(Some(thr)) = decisions.get(file) {
                let ind = " ".repeat(key_col);
                out.push(format!("{ind}noise_gate:"));
                out.push(format!("{ind}  enabled: true"));
                out.push(format!("{ind}  threshold_db: {thr:.1}"));
            }
        }
        i += 1;
    }
    let body = out.join("\n");
    if trailing {
        format!("{body}\n")
    } else {
        body
    }
}

// --- tiny manifest parsers ---

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

fn is_nam_gain_block(t: &str) -> bool {
    matches!(t, "amp" | "preamp" | "gain_pedal")
}

fn all_capture_files(yaml: &str) -> Vec<String> {
    let mut in_caps = false;
    let mut files = Vec::new();
    for line in yaml.lines() {
        let t = line.trim_start();
        if !line.starts_with(char::is_whitespace) && t.starts_with("captures:") {
            in_caps = true;
            continue;
        }
        if in_caps && !line.starts_with(char::is_whitespace) && !t.starts_with('-') && !t.is_empty() {
            break;
        }
        if !in_caps {
            continue;
        }
        let ad = t.strip_prefix("- ").unwrap_or(t);
        if let Some(rest) = ad.strip_prefix("file:") {
            files.push(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_transparent_far_above_threshold() {
        let s = vec![0.3f32; 4800];
        let g = gate(&s, -40.0);
        assert!((rms_dbfs(&g) - rms_dbfs(&s)).abs() < 0.5);
    }

    #[test]
    fn gate_attenuates_far_below_threshold() {
        let n = noise_probe(0.5, -60.0, 48_000);
        let g = gate(&n, -30.0);
        assert!(rms_dbfs(&g) < rms_dbfs(&n) - 6.0);
    }

    #[test]
    fn search_finds_crossing() {
        // pred true above -35 -> smallest_true_rising returns ~-35
        let t = search(|x| x >= -35.0, true);
        assert!((t - -35.0).abs() < 0.5);
    }

    #[test]
    fn upsert_inserts_only_for_gated() {
        let yaml = "type: amp\ncaptures:\n- values:\n    v: a\n  file: captures/a.nam\n- values:\n    v: b\n  file: captures/b.nam\n";
        let mut d = BTreeMap::new();
        d.insert("captures/a.nam".to_string(), Some(-28.0_f32));
        d.insert("captures/b.nam".to_string(), None);
        let out = upsert_capture_noise_gate(yaml, &d);
        assert!(out.contains("  file: captures/a.nam\n  noise_gate:\n    enabled: true\n    threshold_db: -28.0"));
        assert_eq!(out.matches("noise_gate:").count(), 1);
    }

    #[test]
    fn upsert_idempotent_replaces() {
        let y = "type: amp\ncaptures:\n- file: captures/a.nam\n  noise_gate:\n    enabled: true\n    threshold_db: -28.0\n";
        let mut d = BTreeMap::new();
        d.insert("captures/a.nam".to_string(), Some(-31.0_f32));
        let out = upsert_capture_noise_gate(y, &d);
        assert_eq!(out.matches("noise_gate:").count(), 1);
        assert!(out.contains("-31.0") && !out.contains("-28.0"));
    }
}
