//! `nam_loudness_audit` — escreve `output_gain_db` em cada
//! `manifest.yaml` de plugin NAM, calculado a partir do LUFS
//! integrated medido com signal de guitarra sintético determinístico.
//!
//! Issue #413: nivelamento de loudness é metadata estática no
//! manifest. Rodar este binário antes de qualquer release atualiza
//! o offset persistido em cada plugin pra que o app possa aplicar
//! como ganho constante.
//!
//! Estratégia (boost-only, nivela PRA CIMA):
//!   gain = min(TARGET_LUFS - measured_lufs, PEAK_CEILING - measured_peak)
//!         clamped to [0, MAX_GAIN_DB]
//!
//! - Nunca atenua: se o amp já está acima do target (saturated),
//!   manifest gain = 0. O resto do catálogo vem ATÉ ele.
//! - Nunca clipa: se o boost necessário pra atingir LUFS faria o
//!   peak passar do ceiling, o peak vence (clean amps com crest
//!   factor alto não chegam ao mesmo LUFS de saturated por física,
//!   sobem ATÉ o teto).
//!
//! Uso:
//!
//!     cargo run --release -p nam-loudness-audit -- \
//!         /path/to/OpenRig-plugins/plugins/source/nam
//!
//! A escrita preserva ordering / espaçamento do YAML — só substitui
//! ou insere a linha `output_gain_db:` (anchored em `type:`).

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use nam::processor::{close_model_diag, nam_process, open_model_diag};
use nam_loudness_audit::loudness::{
    apply_output_limiter, db_to_lin, integrated_lufs, peak_dbfs,
};
use nam_loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

/// LUFS integrated alvo. -10 LUFS é alto pra streaming standard
/// (-14 LUFS), mas adequado pro contexto: signal de instrumento
/// solo dentro do app, antes do user mixar com batera/baixo. Bate
/// com a média natural dos saturated do catálogo, então nivela
/// PRA CIMA sem precisar atenuar nenhum.
const TARGET_LUFS: f32 = -10.0;

/// Peak ceiling em dBFS após o gain aplicado. +3 dBFS deixa o amp
/// empurrar de leve a zona soft do tanh limiter (a curva começa em
/// 0.95 lin = -0.45 dBFS, fica gentle até ~+3 dBFS, hard saturate
/// só acima disso).
///
/// Por que não -1 dBFS (mais conservador):
/// - Clean amps com crest factor alto (peak >> RMS) batem peak -1
///   muito antes do LUFS chegar no target. Audit clampa, eles
///   ficam quietos demais.
/// - Em +3 dBFS o ceiling vira "boost-tudo-que-der; peaks
///   excedentes o limiter aparada com leve saturação". Trade-off
///   negociado: ~0.5-1 LU de loudness perdida no limiter pra ganhar
///   ~5-10 LU de boost utilizável em clean amps.
const PEAK_CEILING_DBFS: f32 = 3.0;

/// Boost-only: nunca atenua. Se um amp já está acima do target,
/// gain = 0 — o resto do catálogo é que sobe pra alinhar.
const MIN_GAIN_DB: f32 = 0.0;

/// Cap de boost. 30 dB cobre o pior preamp quieto (Fortin Meshuggah)
/// sem deixar capture quebrada (near-silence) explodir o gain.
const MAX_GAIN_DB: f32 = 30.0;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: nam_loudness_audit <plugins-nam-root>");
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
        "target: {TARGET_LUFS:+.2} LUFS, peak ceiling {PEAK_CEILING_DBFS:+.2} dBFS"
    );
    eprintln!("boost-only ({MIN_GAIN_DB:+.0} .. {MAX_GAIN_DB:+.0} dB)");
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
        bail!("type `{block_type}` is not loudness-normalised (only amp/preamp)");
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

    // PRE-limiter measurements: this is what the offline gain math
    // needs (the runtime applies the gain BEFORE the limiter sees
    // the signal, not after).
    let measured_lufs = integrated_lufs(&output, DI_SAMPLE_RATE as u32);
    let measured_peak_dbfs = peak_dbfs(&output);

    let want_for_lufs = TARGET_LUFS - measured_lufs;
    let allowed_by_peak = PEAK_CEILING_DBFS - measured_peak_dbfs;
    let applied = want_for_lufs
        .min(allowed_by_peak)
        .clamp(MIN_GAIN_DB, MAX_GAIN_DB);

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
    matches!(block_type, "amp" | "preamp")
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
    fn loudness_normalisable_only_for_amp_and_preamp() {
        for ok in ["amp", "preamp"] {
            assert!(is_loudness_normalisable(ok));
        }
        for skip in ["gain_pedal", "cab", "body", "reverb", "delay"] {
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
}
