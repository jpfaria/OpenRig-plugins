//! `qa_fix` — re-exports IR `.wav` files so they pass `qa_audit`
//! (issue #12).
//!
//! Per capture: remove DC and resample to 48 kHz with windowed sinc
//! when needed. Then a CEILING-ONLY convolution cap (issue #21):
//! quiet captures pass through at natural level so the boost-only
//! audit (#4) can see their insertion loss; only intrinsically-hot
//! captures whose convolution with the synthetic DI would exceed the
//! ceiling are scaled down — never up.
//!
//! Usage:
//!
//!     cargo run --release -p loudness-audit --bin qa_fix -- \
//!         --source /path/to/OpenRig-plugins/plugins/source
//!
//! Optional `--plugins kind/name[,kind/name…]` (issue #28) restricts
//! the rewrite to a subset — only listed `ir/<name>` plugins are
//! re-processed; everything else is left untouched. `nam/<name>`
//! entries are accepted for symmetry with `qa_audit` but silently
//! ignored (NAM captures are read-only here).
//!
//!     cargo run --release -p loudness-audit --bin qa_fix -- \
//!         --source /path/to/OpenRig-plugins/plugins/source \
//!         --plugins ir/marshall_4x12
//!
//! After running this, `qa_audit` must exit 0 over the same source.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use loudness_audit::ir::convolve;
use loudness_audit::loudness::peak_dbfs;
use loudness_audit::selector::PluginSelector;
use loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};
use loudness_audit::wav_fix::fix_capture;

/// Max convolved peak allowed against the synthetic DI. Above this, the
/// IR is scaled DOWN so qa_audit's CLIP_CEILING (0 dBFS) is not
/// violated downstream. 1 dB of headroom under digital ceiling.
const CONVOLUTION_CEILING_DBFS: f32 = -1.0;

fn main() {
    if let Err(e) = run() {
        eprintln!("qa_fix: {e:#}");
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

    let dst_sr = DI_SAMPLE_RATE as u32;
    let probe = default_guitar_di();

    eprintln!(
        "qa_fix: DC-remove + resample to {dst_sr} Hz + ceiling-only cap at {CONVOLUTION_CEILING_DBFS:+.2} dBFS (#21)"
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

    let mut fixed = 0usize;
    let mut skipped = 0usize;

    let ir_root = source.join("ir");
    if !ir_root.is_dir() {
        bail!("no ir/ subdirectory under {}", source.display());
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(&ir_root)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    for plugin_dir in dirs {
        let manifest = plugin_dir.join("manifest.yaml");
        if !manifest.is_file() {
            continue;
        }
        let plugin_name = plugin_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<?>");
        // Filter-out is silent: the user opted-in to a subset, the
        // non-selected IR plugins are not part of this run's universe.
        if let Some(s) = &selector {
            if !s.matches("ir", plugin_name) {
                continue;
            }
        }
        let raw = fs::read_to_string(&manifest)?;
        for f in all_capture_files(&raw) {
            let path = plugin_dir.join(&f);
            match fix_one(&path, &probe, dst_sr) {
                Ok(FixResult::Fixed) => {
                    fixed += 1;
                    eprintln!("fix  {}", path.display());
                }
                Err(e) => {
                    skipped += 1;
                    eprintln!("skip error  {}: {e:#}", path.display());
                }
            }
        }
    }

    eprintln!();
    eprintln!("qa_fix: fixed={fixed} skipped={skipped}");
    Ok(())
}

enum FixResult {
    Fixed,
}

fn fix_one(path: &Path, probe: &[f32], dst_sr: u32) -> Result<FixResult> {
    let (interleaved, spec) = load_wav_raw(path)?;
    let chans = spec.channels as usize;
    if chans == 0 {
        bail!("zero channels");
    }

    // Deinterleave, fix each channel independently (DC-remove +
    // resample, level preserved), then apply a ceiling-only convolution
    // cap uniformly across channels so stereo balance is preserved.
    let mut channels: Vec<Vec<f32>> = (0..chans)
        .map(|c| {
            let raw: Vec<f32> = interleaved.iter().skip(c).step_by(chans).copied().collect();
            fix_capture(&raw, spec.sample_rate, dst_sr)
        })
        .collect();

    // Max convolved peak across all channels — that's the one that
    // would clip downstream. If it's already below the ceiling, no
    // scaling at all (the boost-only audit needs the natural level).
    let max_peak_db = channels
        .iter()
        .map(|ch| peak_dbfs(&convolve(probe, ch)))
        .filter(|p| p.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    if max_peak_db.is_finite() && max_peak_db > CONVOLUTION_CEILING_DBFS {
        let scale = 10f32.powf((CONVOLUTION_CEILING_DBFS - max_peak_db) / 20.0);
        for ch in channels.iter_mut() {
            for s in ch.iter_mut() {
                *s *= scale;
            }
        }
    }

    let out_len = channels.iter().map(|c| c.len()).max().unwrap_or(0);
    let mut out_interleaved = Vec::with_capacity(out_len * chans);
    for i in 0..out_len {
        for ch in &channels {
            out_interleaved.push(ch.get(i).copied().unwrap_or(0.0));
        }
    }

    let out_spec = hound::WavSpec {
        sample_rate: dst_sr,
        ..spec
    };
    write_wav(path, &out_interleaved, out_spec)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(FixResult::Fixed)
}

fn load_wav_raw(path: &Path) -> Result<(Vec<f32>, hound::WavSpec)> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.samples::<f32>().collect::<Result<_, _>>()?
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };
    Ok((samples, spec))
}

fn write_wav(path: &Path, samples: &[f32], spec: hound::WavSpec) -> Result<()> {
    let mut w = hound::WavWriter::create(path, spec)?;
    match spec.sample_format {
        hound::SampleFormat::Float => {
            for s in samples {
                w.write_sample(*s)?;
            }
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32 - 1.0;
            for s in samples {
                let q = (s * max).round().clamp(-max, max) as i32;
                w.write_sample(q)?;
            }
        }
    }
    w.finalize()?;
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
        "usage: qa_fix --source <plugins/source path> \
         [--plugins kind/name[,kind/name...]]"
    )
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
