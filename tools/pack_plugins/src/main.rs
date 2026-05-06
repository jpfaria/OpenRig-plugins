//! Packs validated plugin packages from `plugins/source/` into release-ready
//! zip archives under `dist/plugins/<backend>/<id>.zip`, plus an
//! `index.json` manifest with sha256 + size for each archive.
//!
//! Usage:
//!
//! ```text
//! pack_plugins [--source <path>] [--dist <path>]
//! ```
//!
//! Defaults: `--source plugins/source`, `--dist dist/plugins`.
//! Exits non-zero when any package fails validation or packing.

use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use plugin_loader::{validate_package, PackageError, PluginManifest};
use serde::Serialize;
use sha2::{Digest, Sha256};
use zip::write::FileOptions;
use zip::CompressionMethod;

const DEFAULT_SOURCE_DIR: &str = "plugins/source";
const DEFAULT_DIST_DIR: &str = "dist/plugins";
const INDEX_FILE_NAME: &str = "index.json";

fn main() -> ExitCode {
    match try_main() {
        Ok(report) => {
            print_report(&report);
            if report.failed.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("pack_plugins: {error}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> anyhow::Result<PackReport> {
    let args = parse_args(std::env::args().skip(1))?;
    let source = args.source.unwrap_or_else(|| PathBuf::from(DEFAULT_SOURCE_DIR));

    if !source.is_dir() {
        anyhow::bail!(
            "source directory `{}` does not exist (use --source <path> to override)",
            source.display()
        );
    }

    if let Some(bundle_path) = args.bundle {
        if let Some(parent) = bundle_path.parent() {
            fs::create_dir_all(parent)?;
        }
        return pack_bundle(&source, &bundle_path);
    }

    let dist = args.dist.unwrap_or_else(|| PathBuf::from(DEFAULT_DIST_DIR));
    fs::create_dir_all(&dist)?;
    let report = pack_all(&source, &dist)?;
    write_index(&dist, &report)?;
    Ok(report)
}

#[derive(Debug, Default)]
struct Args {
    source: Option<PathBuf>,
    dist: Option<PathBuf>,
    bundle: Option<PathBuf>,
}

fn parse_args<I>(iter: I) -> anyhow::Result<Args>
where
    I: IntoIterator<Item = String>,
{
    let mut args = Args::default();
    let mut iter = iter.into_iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--source" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--source needs a path argument"))?;
                args.source = Some(PathBuf::from(value));
            }
            "--dist" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--dist needs a path argument"))?;
                args.dist = Some(PathBuf::from(value));
            }
            "--bundle" => {
                let value = iter
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--bundle needs an output path"))?;
                args.bundle = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                println!(
                    "Usage: pack_plugins [--source <path>] [--dist <path>]\n\
                     Bundle mode: pack_plugins [--source <path>] --bundle <out.zip>\n\
                     Defaults: --source {DEFAULT_SOURCE_DIR} --dist {DEFAULT_DIST_DIR}",
                );
                std::process::exit(0);
            }
            unknown => anyhow::bail!("unknown argument `{unknown}`"),
        }
    }
    Ok(args)
}

#[derive(Debug, Default)]
pub struct PackReport {
    pub packed: Vec<PackEntry>,
    pub failed: Vec<PackFailure>,
}

#[derive(Debug, Serialize)]
pub struct PackEntry {
    pub id: String,
    pub backend: String,
    pub file: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub struct PackFailure {
    pub source_path: PathBuf,
    pub error: PackError,
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("failed to read manifest.yaml: {0}")]
    ReadManifest(#[source] io::Error),
    #[error("invalid manifest.yaml: {0}")]
    ParseManifest(#[source] serde_yaml::Error),
    #[error("validation failed: {0}")]
    Validation(#[source] PackageError),
    #[error("zip creation failed: {0}")]
    Zip(#[source] io::Error),
    #[error("zip library error: {0}")]
    ZipLib(#[from] zip::result::ZipError),
    #[error("missing backend segment in package path `{0}`")]
    MissingBackend(PathBuf),
}

/// Validates every package under `source_root` and writes them all into a
/// single zip at `bundle_path`. Inside the zip, each plugin keeps its
/// `<backend>/<id>/...` layout so the install step can extract straight
/// into the OS-specific plugins dir.
pub fn pack_bundle(source_root: &Path, bundle_path: &Path) -> anyhow::Result<PackReport> {
    let mut report = PackReport::default();
    let mut entries: Vec<PathBuf> = Vec::new();
    walk_packages(source_root, &mut entries)?;
    entries.sort();

    let staging = bundle_path.with_extension("zip.staging");
    if staging.exists() {
        fs::remove_file(&staging)?;
    }
    let file = File::create(&staging)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for source_package in entries {
        let relative = source_package
            .strip_prefix(source_root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| source_package.clone());
        match validate_only(&source_package) {
            Ok(manifest) => {
                if let Err(err) =
                    add_dir_recursive(&mut writer, source_root, &source_package, &options)
                {
                    report.failed.push(PackFailure {
                        source_path: source_package,
                        error: err,
                    });
                    continue;
                }
                report.packed.push(PackEntry {
                    id: manifest.id,
                    backend: relative
                        .components()
                        .next()
                        .and_then(|c| c.as_os_str().to_str())
                        .unwrap_or("")
                        .to_string(),
                    file: relative.to_string_lossy().to_string(),
                    size_bytes: 0,
                    sha256: String::new(),
                });
            }
            Err(error) => report.failed.push(PackFailure {
                source_path: source_package,
                error,
            }),
        }
    }

    writer.finish()?;
    if bundle_path.exists() {
        fs::remove_file(bundle_path)?;
    }
    fs::rename(&staging, bundle_path)?;

    Ok(report)
}

fn validate_only(source_package: &Path) -> Result<PluginManifest, PackError> {
    let manifest_path = source_package.join("manifest.yaml");
    let yaml = fs::read_to_string(&manifest_path).map_err(PackError::ReadManifest)?;
    let manifest: PluginManifest =
        serde_yaml::from_str(&yaml).map_err(PackError::ParseManifest)?;
    validate_package(source_package, &manifest).map_err(PackError::Validation)?;
    Ok(manifest)
}

pub fn pack_all(source_root: &Path, dist_root: &Path) -> io::Result<PackReport> {
    let mut report = PackReport::default();
    let mut entries: Vec<PathBuf> = Vec::new();
    walk_packages(source_root, &mut entries)?;
    entries.sort();

    for source_package in entries {
        let relative = source_package
            .strip_prefix(source_root)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| source_package.clone());
        match pack_one(&source_package, dist_root, &relative) {
            Ok(entry) => report.packed.push(entry),
            Err(error) => report.failed.push(PackFailure {
                source_path: source_package,
                error,
            }),
        }
    }
    Ok(report)
}

fn walk_packages(root: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("manifest.yaml").is_file() {
            out.push(path);
        } else {
            walk_packages(&path, out)?;
        }
    }
    Ok(())
}

fn pack_one(
    source_package: &Path,
    dist_root: &Path,
    relative: &Path,
) -> Result<PackEntry, PackError> {
    let manifest_path = source_package.join("manifest.yaml");
    let yaml = fs::read_to_string(&manifest_path).map_err(PackError::ReadManifest)?;
    let manifest: PluginManifest =
        serde_yaml::from_str(&yaml).map_err(PackError::ParseManifest)?;
    validate_package(source_package, &manifest).map_err(PackError::Validation)?;

    let mut components = relative.components();
    let backend = components
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .ok_or_else(|| PackError::MissingBackend(relative.to_path_buf()))?
        .to_string();
    let id_segment = components
        .next()
        .and_then(|c| c.as_os_str().to_str())
        .unwrap_or(&manifest.id)
        .to_string();

    let backend_dir = dist_root.join(&backend);
    fs::create_dir_all(&backend_dir).map_err(PackError::Zip)?;

    let zip_path = backend_dir.join(format!("{id_segment}.zip"));
    let staging_path = backend_dir.join(format!(".staging.{id_segment}.zip"));
    if staging_path.exists() {
        fs::remove_file(&staging_path).map_err(PackError::Zip)?;
    }

    write_zip(source_package, &staging_path)?;
    if zip_path.exists() {
        fs::remove_file(&zip_path).map_err(PackError::Zip)?;
    }
    fs::rename(&staging_path, &zip_path).map_err(PackError::Zip)?;

    let (size_bytes, sha256) = hash_file(&zip_path).map_err(PackError::Zip)?;
    let file = format!("{backend}/{id_segment}.zip");

    Ok(PackEntry {
        id: manifest.id,
        backend,
        file,
        size_bytes,
        sha256,
    })
}

fn write_zip(source_dir: &Path, out_path: &Path) -> Result<(), PackError> {
    let file = File::create(out_path).map_err(PackError::Zip)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(0o644);

    add_dir_recursive(&mut writer, source_dir, source_dir, &options)?;
    writer.finish()?;
    Ok(())
}

fn add_dir_recursive<W: Write + io::Seek>(
    writer: &mut zip::ZipWriter<W>,
    root: &Path,
    current: &Path,
    options: &FileOptions,
) -> Result<(), PackError> {
    for entry in fs::read_dir(current).map_err(PackError::Zip)? {
        let entry = entry.map_err(PackError::Zip)?;
        let entry_type = entry.file_type().map_err(PackError::Zip)?;
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .expect("entry inside root")
            .to_string_lossy()
            .replace('\\', "/");
        if entry_type.is_dir() {
            writer.add_directory(format!("{rel}/"), *options)?;
            add_dir_recursive(writer, root, &path, options)?;
        } else if entry_type.is_file() {
            writer.start_file(rel, *options)?;
            let mut input = File::open(&path).map_err(PackError::Zip)?;
            io::copy(&mut input, writer).map_err(PackError::Zip)?;
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    let digest = hasher.finalize();
    let hex = digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    });
    Ok((total, hex))
}

#[derive(Debug, Serialize)]
struct Index<'a> {
    schema_version: u32,
    plugins: &'a [PackEntry],
}

fn write_index(dist_root: &Path, report: &PackReport) -> io::Result<()> {
    let index = Index {
        schema_version: 1,
        plugins: &report.packed,
    };
    let path = dist_root.join(INDEX_FILE_NAME);
    let json = serde_json::to_string_pretty(&index)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(&path, json)?;
    Ok(())
}

fn print_report(report: &PackReport) {
    println!(
        "pack_plugins: packed {} archive(s), {} failed",
        report.packed.len(),
        report.failed.len()
    );
    for entry in &report.packed {
        if entry.sha256.is_empty() {
            println!("  ok    {}", entry.file);
        } else {
            println!(
                "  ok    {}  ({} bytes, {})",
                entry.file,
                entry.size_bytes,
                &entry.sha256[..12]
            );
        }
    }
    for failure in &report.failed {
        let label = failure
            .source_path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("<unknown>");
        eprintln!("  FAIL  {label}: {}", failure.error);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "openrig-pack-{label}-{}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp");
            Self { path }
        }

        fn write(&self, relative: &str, contents: &[u8]) {
            let absolute = self.path.join(relative);
            if let Some(parent) = absolute.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(&absolute, contents).expect("write file");
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_valid_nam_source(tmp: &TempDir, package_id: &str) {
        let manifest = format!(
            r#"manifest_version: 1
id: {package_id}
display_name: {package_id}
type: preamp
backend: nam
parameters:
  - name: gain
    values: [10]
captures:
  - values: {{ gain: 10 }}
    file: captures/g10.nam
"#
        );
        tmp.write(
            &format!("source/nam/{package_id}/manifest.yaml"),
            manifest.as_bytes(),
        );
        tmp.write(
            &format!("source/nam/{package_id}/captures/g10.nam"),
            b"fake nam bytes",
        );
    }

    #[test]
    fn pack_one_package_writes_zip_and_index_entry() {
        let tmp = TempDir::new("one");
        write_valid_nam_source(&tmp, "alpha");

        let source = tmp.path.join("source");
        let dist = tmp.path.join("dist");
        let report = pack_all(&source, &dist).expect("pack");

        assert!(report.failed.is_empty());
        assert_eq!(report.packed.len(), 1);
        let entry = &report.packed[0];
        assert_eq!(entry.id, "alpha");
        assert_eq!(entry.backend, "nam");
        assert_eq!(entry.file, "nam/alpha.zip");
        assert!(dist.join("nam/alpha.zip").is_file());
        assert!(entry.size_bytes > 0);
        assert_eq!(entry.sha256.len(), 64);
    }

    #[test]
    fn pack_is_idempotent_and_replaces_existing_zip() {
        let tmp = TempDir::new("idempotent");
        write_valid_nam_source(&tmp, "alpha");

        let source = tmp.path.join("source");
        let dist = tmp.path.join("dist");

        let _ = pack_all(&source, &dist).expect("first pack");
        let report = pack_all(&source, &dist).expect("second pack");

        assert_eq!(report.packed.len(), 1);
        assert!(dist.join("nam/alpha.zip").is_file());
    }

    #[test]
    fn invalid_manifest_is_collected_without_blocking_others() {
        let tmp = TempDir::new("mixed");
        write_valid_nam_source(&tmp, "good");
        tmp.write(
            "source/lv2/bad/manifest.yaml",
            br#"manifest_version: 1
id: bad
display_name: Bad
type: util
backend: lv2
plugin_uri: ""
bundle_path: bundles/bad.lv2
binaries:
  linux-x86_64: bundles/bad.lv2/linux-x86_64/plugin.so
"#,
        );

        let source = tmp.path.join("source");
        let dist = tmp.path.join("dist");
        let report = pack_all(&source, &dist).expect("pack");

        assert_eq!(report.packed.len(), 1);
        assert_eq!(report.failed.len(), 1);
        assert!(matches!(report.failed[0].error, PackError::Validation(_)));
        assert!(dist.join("nam/good.zip").is_file());
        assert!(!dist.join("lv2/bad.zip").exists());
    }

    #[test]
    fn parse_args_accepts_source_and_dist_overrides() {
        let args = parse_args(
            ["--source", "/tmp/src", "--dist", "/tmp/d"]
                .into_iter()
                .map(String::from),
        )
        .expect("parse");
        assert_eq!(args.source, Some(PathBuf::from("/tmp/src")));
        assert_eq!(args.dist, Some(PathBuf::from("/tmp/d")));
    }

    #[test]
    fn parse_args_rejects_unknown_flag() {
        assert!(parse_args(["--what".to_string()]).is_err());
    }
}
