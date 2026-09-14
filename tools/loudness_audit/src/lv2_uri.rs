//! LV2 URI consistency check (issue #133).
//!
//! OpenRig loads an LV2 package by walking `lv2_descriptor(i)` in the
//! slot binary until it finds the manifest's `plugin_uri`, and reads the
//! ports from the `data/*.ttl` subject with that same URI. When an
//! upstream rebuild changes the URI (mda-lv2: `moddevices.com` →
//! `drobilla.net`) and the manifest/TTL are not updated, every package
//! fails to instantiate while packing stays green. This check makes the
//! three sources agree:
//!
//! - every `binaries:` file contains `plugin_uri` as a NUL-terminated
//!   string (the descriptor URI is a C string literal, so a static byte
//!   scan works for every slot, cross-platform, without loading it);
//! - some `data/*.ttl` declares `plugin_uri` as `a lv2:Plugin`, either as
//!   `<uri>` or as a prefixed name expanded from `@prefix`.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Top-level `plugin_uri:` of an LV2 `manifest.yaml`.
pub fn manifest_plugin_uri(yaml: &str) -> Option<String> {
    yaml.lines()
        .filter(|l| !l.starts_with(char::is_whitespace))
        .find_map(|l| l.strip_prefix("plugin_uri:"))
        .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|v| !v.is_empty())
}

/// `(slot, relative path)` pairs of the manifest's `binaries:` map.
pub fn manifest_binaries(yaml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_binaries = false;
    for line in yaml.lines() {
        if !line.starts_with(char::is_whitespace) {
            in_binaries = line.trim_end() == "binaries:";
            continue;
        }
        if !in_binaries {
            continue;
        }
        if let Some((slot, path)) = line.trim().split_once(':') {
            let path = path.trim().trim_matches('"').trim_matches('\'');
            if !slot.is_empty() && !path.is_empty() {
                out.push((slot.trim().to_string(), path.to_string()));
            }
        }
    }
    out
}

/// True when `binary` carries `uri` as a complete C string. The NUL
/// terminator keeps `…/Detune` from matching inside `…/Detune2`.
pub fn binary_publishes_uri(binary: &[u8], uri: &str) -> bool {
    let mut needle = uri.as_bytes().to_vec();
    needle.push(0);
    binary.windows(needle.len()).any(|w| w == needle.as_slice())
}

/// True when `ttl` declares `uri` as the subject of `a lv2:Plugin`.
pub fn ttl_declares_plugin(ttl: &str, uri: &str) -> bool {
    let mut candidates = vec![format!("<{uri}>")];
    for (name, base) in turtle_prefixes(ttl) {
        if let Some(local) = uri.strip_prefix(base.as_str()) {
            if !local.is_empty() {
                candidates.push(format!("{name}:{local}"));
            }
        }
    }
    candidates
        .iter()
        .any(|c| subject_occurrences(ttl, c).any(|rest| declares_plugin_type(rest)))
}

/// Text following each occurrence of `term` that stands as a whole token.
fn subject_occurrences<'a>(ttl: &'a str, term: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    ttl.match_indices(term).filter_map(move |(idx, _)| {
        let before_ok = ttl[..idx]
            .chars()
            .next_back()
            .map_or(true, char::is_whitespace);
        let rest = &ttl[idx + term.len()..];
        let after_ok = rest.chars().next().map_or(false, char::is_whitespace);
        (before_ok && after_ok).then_some(rest)
    })
}

/// `rest` opens with `a <types>` and that type list contains `lv2:Plugin`.
fn declares_plugin_type(rest: &str) -> bool {
    let Some(types) = rest.trim_start().strip_prefix('a') else {
        return false;
    };
    if !types.starts_with(char::is_whitespace) {
        return false;
    }
    let end = types.find([';', '.']).unwrap_or(types.len());
    types[..end]
        .split(|c: char| c == ',' || c.is_whitespace())
        .any(|t| t == "lv2:Plugin")
}

/// `@prefix name: <base> .` declarations, in document order.
fn turtle_prefixes(ttl: &str) -> Vec<(String, String)> {
    ttl.lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("@prefix")?;
            let (name, rest) = rest.split_once(':')?;
            let base = rest.split_once('<')?.1.split_once('>')?.0;
            Some((name.trim().to_string(), base.to_string()))
        })
        .collect()
}

/// Every URI inconsistency of the LV2 package at `plugin_dir`; empty
/// when the manifest, binaries and TTLs agree.
pub fn check_lv2_package(plugin_dir: &Path) -> Result<Vec<String>> {
    let manifest = plugin_dir.join("manifest.yaml");
    let yaml = fs::read_to_string(&manifest)
        .with_context(|| format!("read {}", manifest.display()))?;
    let Some(uri) = manifest_plugin_uri(&yaml) else {
        return Ok(vec!["manifest has no plugin_uri".into()]);
    };

    let mut fails = Vec::new();
    for (slot, rel) in manifest_binaries(&yaml) {
        match fs::read(plugin_dir.join(&rel)) {
            Ok(bytes) if binary_publishes_uri(&bytes, &uri) => {}
            Ok(_) => fails.push(format!("{slot}: `{rel}` does not publish {uri}")),
            Err(e) => fails.push(format!("{slot}: cannot read `{rel}`: {e}")),
        }
    }

    let mut declared = false;
    if let Ok(entries) = fs::read_dir(plugin_dir.join("data")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("ttl") {
                continue;
            }
            if let Ok(ttl) = fs::read_to_string(&path) {
                declared |= ttl_declares_plugin(&ttl, &uri);
            }
        }
    }
    if !declared {
        fails.push(format!("no data/*.ttl declares {uri} as `a lv2:Plugin`"));
    }
    Ok(fails)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const DROBILLA: &str = "http://drobilla.net/plugins/mda/DubDelay";
    const MODDEVICES: &str = "http://moddevices.com/plugins/mda/DubDelay";

    const UPSTREAM_TTL: &str = "@prefix lv2: <http://lv2plug.in/ns/lv2core#> .\n\
        @prefix mda: <http://drobilla.net/plugins/mda/> .\n\n\
        mda:DubDelay\n\ta lv2:Plugin ,\n\t\tlv2:DelayPlugin ;\n\tlv2:symbol \"DubDelay\" .\n";

    fn binary_with(uri: &str) -> Vec<u8> {
        let mut b = b"\x7fELF junk\0http://lv2plug.in/ns/ext/urid#map\0".to_vec();
        b.extend_from_slice(uri.as_bytes());
        b.extend_from_slice(b"\0more junk");
        b
    }

    #[test]
    fn binary_with_manifest_uri_passes() {
        assert!(binary_publishes_uri(&binary_with(DROBILLA), DROBILLA));
    }

    #[test]
    fn binary_with_other_host_uri_fails() {
        // The #133 shape: rebuilt binary publishes drobilla, manifest says moddevices.
        assert!(!binary_publishes_uri(&binary_with(DROBILLA), MODDEVICES));
    }

    #[test]
    fn binary_uri_must_be_a_whole_string() {
        let b = binary_with("http://drobilla.net/plugins/mda/Detune2");
        assert!(!binary_publishes_uri(&b, "http://drobilla.net/plugins/mda/Detune"));
    }

    #[test]
    fn ttl_prefixed_declaration_passes() {
        assert!(ttl_declares_plugin(UPSTREAM_TTL, DROBILLA));
    }

    #[test]
    fn ttl_absolute_declaration_passes() {
        let ttl = format!("<{DROBILLA}>\n    a lv2:Plugin ;\n    lv2:symbol \"x\" .\n");
        assert!(ttl_declares_plugin(&ttl, DROBILLA));
    }

    #[test]
    fn ttl_declaring_another_prefix_fails() {
        assert!(!ttl_declares_plugin(UPSTREAM_TTL, MODDEVICES));
    }

    #[test]
    fn ttl_reference_that_is_not_a_declaration_fails() {
        let preset = format!("<urn:p>\n\ta pset:Preset ;\n\tlv2:appliesTo <{DROBILLA}> .\n");
        assert!(!ttl_declares_plugin(&preset, DROBILLA));
    }

    #[test]
    fn ttl_longer_local_name_does_not_match() {
        let ttl = UPSTREAM_TTL.replace("mda:DubDelay\n", "mda:DubDelayX\n");
        assert!(!ttl_declares_plugin(&ttl, DROBILLA));
    }

    #[test]
    fn ttl_plugin_subclass_type_is_not_plugin() {
        let ttl = format!("<{DROBILLA}>\n    a lv2:PluginBase ;\n    lv2:symbol \"x\" .\n");
        assert!(!ttl_declares_plugin(&ttl, DROBILLA));
    }

    #[test]
    fn parses_manifest_uri_and_binaries() {
        let yaml = "id: x\nplugin_uri: http://a/b\nbinaries:\n  macos-universal: platform/macos-universal/B.dylib\n  linux-x86_64: platform/linux-x86_64/B.so\nparameters:\n  - symbol: gain\n";
        assert_eq!(manifest_plugin_uri(yaml).as_deref(), Some("http://a/b"));
        assert_eq!(
            manifest_binaries(yaml),
            vec![
                ("macos-universal".to_string(), "platform/macos-universal/B.dylib".to_string()),
                ("linux-x86_64".to_string(), "platform/linux-x86_64/B.so".to_string()),
            ]
        );
    }

    struct TempPackage(PathBuf);

    impl TempPackage {
        fn new(label: &str, manifest_uri: &str, binary_uri: &str, ttl: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("openrig-lv2-uri-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("platform/linux-x86_64")).unwrap();
            fs::create_dir_all(dir.join("data")).unwrap();
            fs::write(
                dir.join("manifest.yaml"),
                format!("backend: lv2\nplugin_uri: {manifest_uri}\nbinaries:\n  linux-x86_64: platform/linux-x86_64/DubDelay.so\n"),
            )
            .unwrap();
            fs::write(dir.join("platform/linux-x86_64/DubDelay.so"), binary_with(binary_uri)).unwrap();
            fs::write(dir.join("data/DubDelay.ttl"), ttl).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempPackage {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn consistent_package_passes() {
        let pkg = TempPackage::new("ok", DROBILLA, DROBILLA, UPSTREAM_TTL);
        assert_eq!(check_lv2_package(&pkg.0).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn stale_manifest_uri_fails_on_binary_and_ttl() {
        let pkg = TempPackage::new("stale", MODDEVICES, DROBILLA, UPSTREAM_TTL);
        let fails = check_lv2_package(&pkg.0).unwrap();
        assert_eq!(fails.len(), 2, "{fails:?}");
        assert!(fails[0].starts_with("linux-x86_64:"), "{fails:?}");
        assert!(fails[1].contains("no data/*.ttl declares"), "{fails:?}");
    }
}
