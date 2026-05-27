//! Optional `--plugins kind/name[,kind/name…]` selector shared by the
//! `qa_audit` and `qa_fix` binaries (issue #28).
//!
//! Default behaviour (no flag) is preserved — both binaries walk every
//! plugin under `--source`. When the flag is present they restrict the
//! universe to the listed `<kind>/<name>` entries, enabling fast
//! iteration on a single plugin without reprocessing the whole tree.
//!
//! Only the two on-disk roots are accepted as kinds: `nam` and `ir`.
//! Anything else is a parse error so a typo is caught up-front instead
//! of silently filtering everything out.

use anyhow::{anyhow, bail, Result};
use std::path::Path;

/// Roots that exist under `plugins/source/`. Single source of truth for
/// the allowed `kind` token in `--plugins kind/name`.
pub const ALLOWED_KINDS: [&str; 2] = ["nam", "ir"];

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PluginSelector {
    entries: Vec<(String, String)>,
}

impl PluginSelector {
    /// Parses a `--plugins` value: `kind/name[,kind/name…]`.
    ///
    /// Empty input (after trimming and stripping commas) is an error so
    /// the caller can never end up with a selector that matches
    /// nothing. Duplicates are deduped — `nam/x,nam/x` is the same
    /// universe as `nam/x`.
    pub fn parse(raw: &str) -> Result<Self> {
        let mut entries: Vec<(String, String)> = Vec::new();
        for token in raw.split(',') {
            let t = token.trim();
            if t.is_empty() {
                continue;
            }
            let (kind, name) = t
                .split_once('/')
                .ok_or_else(|| anyhow!("--plugins entry '{t}' is missing '/' (expected kind/name)"))?;
            let kind = kind.trim();
            let name = name.trim();
            if kind.is_empty() || name.is_empty() {
                bail!("--plugins entry '{t}' has an empty kind or name");
            }
            if !ALLOWED_KINDS.contains(&kind) {
                bail!(
                    "--plugins entry '{t}' has unknown kind '{kind}' (allowed: {})",
                    ALLOWED_KINDS.join(", ")
                );
            }
            let pair = (kind.to_string(), name.to_string());
            if !entries.contains(&pair) {
                entries.push(pair);
            }
        }
        if entries.is_empty() {
            bail!("--plugins is empty");
        }
        Ok(Self { entries })
    }

    /// Scans `args` for `--plugins <value>`. Returns `Ok(None)` when the
    /// flag is absent so callers can preserve their full-walk default
    /// without a special case.
    pub fn from_args(args: &[String]) -> Result<Option<Self>> {
        let mut it = args.iter().skip(1);
        while let Some(a) = it.next() {
            if a == "--plugins" {
                let v = it
                    .next()
                    .ok_or_else(|| anyhow!("--plugins requires a value"))?;
                return Self::parse(v).map(Some);
            }
        }
        Ok(None)
    }

    pub fn matches(&self, kind: &str, name: &str) -> bool {
        self.entries
            .iter()
            .any(|(k, n)| k == kind && n == name)
    }

    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    /// Fails if any selected `(kind, name)` is missing the expected
    /// `plugins/source/<kind>/<name>/manifest.yaml`. Run this once,
    /// before any audit/fix work, so typos abort immediately instead of
    /// producing a silently-empty walk.
    pub fn validate_against(&self, source: &Path) -> Result<()> {
        for (kind, name) in &self.entries {
            let manifest = source.join(kind).join(name).join("manifest.yaml");
            if !manifest.is_file() {
                bail!("selector not found: {}", manifest.display());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ok_single() {
        let s = PluginSelector::parse("nam/mesa_rectifier").unwrap();
        assert_eq!(s.entries(), &[("nam".into(), "mesa_rectifier".into())]);
        assert!(s.matches("nam", "mesa_rectifier"));
        assert!(!s.matches("ir", "mesa_rectifier"));
        assert!(!s.matches("nam", "ibanez_ts9"));
    }

    #[test]
    fn parse_ok_multi_both_kinds() {
        let s = PluginSelector::parse("nam/foo,ir/bar").unwrap();
        assert_eq!(s.entries().len(), 2);
        assert!(s.matches("nam", "foo"));
        assert!(s.matches("ir", "bar"));
    }

    #[test]
    fn parse_dedups() {
        let s = PluginSelector::parse("nam/x,nam/x,nam/x").unwrap();
        assert_eq!(s.entries(), &[("nam".into(), "x".into())]);
    }

    #[test]
    fn parse_trims_whitespace_around_tokens() {
        let s = PluginSelector::parse(" nam/foo , ir/bar ").unwrap();
        assert!(s.matches("nam", "foo"));
        assert!(s.matches("ir", "bar"));
    }

    #[test]
    fn parse_err_empty_value() {
        assert!(PluginSelector::parse("").is_err());
        assert!(PluginSelector::parse(",,, ").is_err());
    }

    #[test]
    fn parse_err_unknown_kind() {
        let e = PluginSelector::parse("lv2/foo").unwrap_err().to_string();
        assert!(e.contains("unknown kind 'lv2'"), "msg was: {e}");
    }

    #[test]
    fn parse_err_missing_slash() {
        let e = PluginSelector::parse("just_a_name").unwrap_err().to_string();
        assert!(e.contains("missing '/'"), "msg was: {e}");
    }

    #[test]
    fn parse_err_empty_kind_or_name() {
        assert!(PluginSelector::parse("nam/").is_err());
        assert!(PluginSelector::parse("/foo").is_err());
    }

    #[test]
    fn from_args_absent_returns_none() {
        let args = vec!["qa_audit".into(), "--source".into(), "/x".into()];
        assert!(PluginSelector::from_args(&args).unwrap().is_none());
    }

    #[test]
    fn from_args_present_parses_value() {
        let args = vec![
            "qa_audit".into(),
            "--source".into(),
            "/x".into(),
            "--plugins".into(),
            "nam/foo".into(),
        ];
        let s = PluginSelector::from_args(&args).unwrap().unwrap();
        assert!(s.matches("nam", "foo"));
    }

    #[test]
    fn from_args_present_without_value_errors() {
        let args = vec!["qa_audit".into(), "--plugins".into()];
        assert!(PluginSelector::from_args(&args).is_err());
    }
}
