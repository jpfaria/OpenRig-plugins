//! Minimal manifest walker for the audit binary AND the LUFS test.
//!
//! NÃO usa serde_yaml: o manifest tem variantes (params livres, etc)
//! que exigiriam tipos completos de `plugin-loader`. Pra `type` /
//! primeiro `captures.file` / `output_gain_db`, line-grep é suficiente
//! e zero deps extras. Mesma estratégia do binário de audit.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub plugin_dir: PathBuf,
    pub plugin_id: String,
    pub block_type: String,
    pub capture_path: PathBuf,
    pub output_gain_db: Option<f32>,
}

pub fn list_loudness_normalisable(root: &Path) -> Result<Vec<CatalogEntry>> {
    if !root.is_dir() {
        bail!("not a directory: {}", root.display());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();

    let mut out = Vec::new();
    for plugin_dir in entries {
        let manifest = plugin_dir.join("manifest.yaml");
        if !manifest.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&manifest)
            .with_context(|| format!("read {}", manifest.display()))?;
        let block_type = manifest_field(&raw, "type").unwrap_or_default();
        if !is_loudness_normalisable(&block_type) {
            continue;
        }
        let capture_file = first_capture_file(&raw)
            .ok_or_else(|| anyhow!("{}: no captures[].file", manifest.display()))?;
        let output_gain_db = manifest_field(&raw, "output_gain_db").and_then(|s| s.parse().ok());
        let plugin_id = plugin_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<?>")
            .to_string();
        out.push(CatalogEntry {
            plugin_id,
            plugin_dir: plugin_dir.clone(),
            block_type,
            capture_path: plugin_dir.join(capture_file),
            output_gain_db,
        });
    }
    Ok(out)
}

/// Mirrors `main.rs::is_loudness_normalisable`. Tipos cujo
/// `output_gain_db` é populado pelo audit pra preservar volume
/// quando o bloco entra/sai da chain.
fn is_loudness_normalisable(block_type: &str) -> bool {
    matches!(block_type, "amp" | "preamp" | "gain_pedal")
}

/// Reads a top-level scalar field from the manifest (`field: value`).
/// Returns None if missing.
fn manifest_field(yaml: &str, field: &str) -> Option<String> {
    let prefix = format!("{field}:");
    for line in yaml.lines() {
        // Top-level only: skip indented lines (no leading whitespace).
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(rest) = line.strip_prefix(&prefix) {
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
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
            return Some(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_top_level_scalar_field() {
        let y = "id: nam_test\ntype: amp\nbackend: nam\noutput_gain_db: 7.5\n";
        assert_eq!(manifest_field(y, "type"), Some("amp".to_string()));
        assert_eq!(manifest_field(y, "output_gain_db"), Some("7.5".to_string()));
    }

    #[test]
    fn ignores_indented_field_with_same_name() {
        let y = "id: x\nbackend: nam\nbackend_config:\n  type: nested\ntype: amp\n";
        // Should pick the top-level `type: amp`, not the indented one.
        assert_eq!(manifest_field(y, "type"), Some("amp".to_string()));
    }

    #[test]
    fn missing_field_returns_none() {
        assert_eq!(manifest_field("id: x\n", "output_gain_db"), None);
    }

    #[test]
    fn first_capture_file_handles_quotes() {
        let y = "captures:\n- values: {}\n  file: \"captures/foo bar.nam\"\n";
        assert_eq!(
            first_capture_file(y),
            Some("captures/foo bar.nam".to_string())
        );
    }
}
