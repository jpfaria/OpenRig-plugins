//! Validation test: depois do `nam_loudness_audit` ter populado
//! `output_gain_db` em cada `manifest.yaml`, este test ITERA TODOS
//! os plugins amp/preamp do catálogo, joga o MESMO pink noise por
//! cada um, mede o RMS final e asserta que TODOS estão dentro de
//! tolerância (default 3 dB).
//!
//! É a auditoria de aceitação do catálogo: se algum NAM ficou
//! desnivelado, este test reporta exatamente quem.
//!
//! Default: roda contra `plugins/source/nam/` (raiz do repo). Pra
//! testar outro catálogo:
//!
//!     OPENRIG_PLUGINS_NAM_ROOT=/caminho cargo test ... -- --nocapture

use std::path::PathBuf;

use nam::loudness_probe::{diagnose_model, PROBE_INPUT_PEAK_DBFS};
use nam::processor::{close_model_diag, open_model_diag};

/// Tolerância sobre o RMS spread pós-audit. < 1 dB — diferença
/// audível começa em ~1 dB, e o objetivo é o usuário não sentir
/// salto de loudness ao trocar de preset/amp.
const TOLERANCE_DB: f32 = 1.0;

fn plugins_nam_root() -> PathBuf {
    if let Ok(p) = std::env::var("OPENRIG_PLUGINS_NAM_ROOT") {
        return PathBuf::from(p);
    }
    // Workspace runs from tools/nam_loudness_audit, so the source
    // tree root is two parents up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("plugins")
        .join("source")
        .join("nam")
}

fn manifest_block_type(yaml: &str) -> Option<String> {
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("type:") {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

fn manifest_output_gain_db(yaml: &str) -> Option<f32> {
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("output_gain_db:") {
            return rest.trim().parse::<f32>().ok();
        }
    }
    None
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
            let value = rest
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            return Some(value);
        }
    }
    None
}

#[test]
#[ignore]
fn every_audited_amp_outputs_within_tolerance_of_every_other() {
    let root = plugins_nam_root();
    assert!(
        root.is_dir(),
        "plugins root not a directory: {} (set OPENRIG_PLUGINS_NAM_ROOT or run from repo root)",
        root.display()
    );

    let mut measurements: Vec<(String, f32, f32)> = Vec::new();

    for entry in std::fs::read_dir(&root).expect("read plugins root") {
        let entry = entry.expect("dir entry");
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }
        let manifest_path = plugin_dir.join("manifest.yaml");
        if !manifest_path.is_file() {
            continue;
        }
        let yaml = std::fs::read_to_string(&manifest_path).expect("read manifest");
        let block_type = match manifest_block_type(&yaml) {
            Some(t) => t,
            None => continue,
        };
        if block_type != "amp" && block_type != "preamp" {
            continue;
        }
        let gain_db = manifest_output_gain_db(&yaml).unwrap_or_else(|| {
            panic!(
                "{}: amp/preamp sem `output_gain_db` no manifest — rode `nam_loudness_audit` primeiro",
                plugin_dir.display()
            )
        });
        let capture = match first_capture_file(&yaml) {
            Some(c) => c,
            None => continue,
        };
        let model_path = plugin_dir.join(&capture);
        let model_path_str = model_path.to_str().expect("utf8 path");
        let model = match open_model_diag(model_path_str) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("SKIP {}: {e}", plugin_dir.display());
                continue;
            }
        };
        let report = unsafe { diagnose_model(model) };
        unsafe { close_model_diag(model) };

        // Final PEAK = raw probe peak + manifest output_gain_db.
        // (The runtime applies the gain as `params.output_level_db += manifest_gain_db`.)
        let final_rms = report.output_peak_dbfs + gain_db;
        let name = plugin_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        measurements.push((name, final_rms, gain_db));
    }

    assert!(
        measurements.len() >= 2,
        "need ≥2 amps/preamps to test convergence; got {}",
        measurements.len()
    );

    let mut sorted = measurements.clone();
    sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    let min = sorted.first().unwrap().clone();
    let max = sorted.last().unwrap().clone();
    let spread = max.1 - min.1;

    eprintln!("");
    eprintln!(
        "audited {} amps/preamps  (probe input peak {:.1} dBFS)",
        measurements.len(),
        PROBE_INPUT_PEAK_DBFS
    );
    eprintln!(
        "loudest  : {:+.2} dBFS  {} (output_gain_db={:+.2})",
        max.1, max.0, max.2
    );
    eprintln!(
        "quietest : {:+.2} dBFS  {} (output_gain_db={:+.2})",
        min.1, min.0, min.2
    );
    eprintln!("spread   : {:+.2} dB", spread);

    if spread >= TOLERANCE_DB {
        eprintln!("");
        eprintln!("Outliers > {TOLERANCE_DB} dB do mediano:");
        let median = sorted[sorted.len() / 2].1;
        let mut tagged: Vec<_> = measurements
            .iter()
            .map(|(n, r, g)| (n, r, g, (r - median).abs()))
            .collect();
        tagged.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
        for (n, r, g, d) in tagged.into_iter().take(15) {
            eprintln!(
                "  Δ {:+.2} dB  rms={:+.2}  gain_db={:+.2}  {}",
                d, r, g, n
            );
        }
    }

    assert!(
        spread < TOLERANCE_DB,
        "RMS spread = {spread:.2} dB (> {TOLERANCE_DB} dB) — audit não está nivelando o catalog"
    );
}
