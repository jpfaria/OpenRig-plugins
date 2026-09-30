//! Plugin info metadata check (issue #153).
//!
//! OpenRig's plugin info panel shows the manifest's `description`,
//! `license` and `homepage` when the user clicks a block. A NAM/IR
//! package without them renders an empty panel, and nothing in packing
//! notices. This check makes the metadata part of the gate:
//!
//! - every NAM/IR manifest carries a non-empty top-level `description`;
//! - a manifest that declares `sources:` also carries a `homepage`, so
//!   the panel links to where the capture came from.

/// Non-empty value of a top-level `key:` scalar in a manifest.
pub fn top_level_scalar(yaml: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    yaml.lines()
        .filter(|l| !l.starts_with(char::is_whitespace))
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|v| !v.is_empty())
}

/// True when the manifest has a top-level `sources:` list with at least
/// one entry.
fn has_sources(yaml: &str) -> bool {
    let mut lines = yaml.lines().skip_while(|l| l.trim_end() != "sources:");
    if lines.next().is_none() {
        return false;
    }
    lines
        .next()
        .is_some_and(|l| l.trim_start().starts_with("- ") && l.trim().len() > 2)
}

/// Missing-metadata failures of a NAM/IR manifest; empty when complete.
pub fn missing_metadata(yaml: &str) -> Vec<String> {
    let mut fails = Vec::new();
    if top_level_scalar(yaml, "description").is_none() {
        fails.push("missing description".to_string());
    }
    if has_sources(yaml) && top_level_scalar(yaml, "homepage").is_none() {
        fails.push("has sources but missing homepage".to_string());
    }
    fails
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE: &str = "id: nam_x\n\
        display_name: X\n\
        description: Neural capture of the X amp.\n\
        homepage: https://www.tone3000.com/tones/1\n\
        sources:\n\
        - https://www.tone3000.com/tones/1\n\
        parameters: []\n";

    #[test]
    fn complete_manifest_passes() {
        assert!(missing_metadata(COMPLETE).is_empty());
    }

    #[test]
    fn missing_description_fails() {
        let yaml = COMPLETE.replace("description: Neural capture of the X amp.\n", "");
        assert_eq!(missing_metadata(&yaml), vec!["missing description"]);
    }

    #[test]
    fn empty_description_fails() {
        let yaml = COMPLETE.replace("Neural capture of the X amp.", "");
        assert_eq!(missing_metadata(&yaml), vec!["missing description"]);
    }

    #[test]
    fn sources_without_homepage_fails() {
        let yaml = COMPLETE.replace("homepage: https://www.tone3000.com/tones/1\n", "");
        assert_eq!(missing_metadata(&yaml), vec!["has sources but missing homepage"]);
    }

    #[test]
    fn no_sources_does_not_require_homepage() {
        let yaml = "id: ir_x\ndescription: Body IR of the X guitar.\nparameters: []\n";
        assert!(missing_metadata(yaml).is_empty());
    }

    #[test]
    fn indented_description_is_not_top_level() {
        let yaml = "id: ir_x\nparameters:\n- name: mic\n  description: nested\n";
        assert_eq!(missing_metadata(yaml), vec!["missing description"]);
    }
}
