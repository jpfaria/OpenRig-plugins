//! `qa_fix` — re-exports IR `.wav` files so they pass `qa_audit`
//! (issue #12).
//!
//! Per capture: remove DC, resample to 48 kHz with windowed sinc if
//! needed, scale so the synthetic-DI convolution peaks at the target
//! ceiling. Mono only — stereo captures are skipped with a clear
//! report so they can be triaged manually.
//!
//! Usage:
//!
//!     cargo run --release -p loudness-audit --bin qa_fix -- \
//!         --source /path/to/OpenRig-plugins/plugins/source
//!
//! After running this, `qa_audit` must exit 0 over the same source.

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};
use loudness_audit::wav_fix::{dc_remove, peak_normalize_for_convolution, sinc_resample};

/// IR is scaled so the convolved probe peaks here. 1 dB safety margin
/// under digital ceiling.
const TARGET_PEAK_DBFS: f32 = -1.0;

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

    let di = default_guitar_di();
    let dst_sr = DI_SAMPLE_RATE as u32;

    eprintln!("qa_fix: target peak {TARGET_PEAK_DBFS:+.2} dBFS, target SR {dst_sr} Hz");
    eprintln!("source: {}", source.display());
    eprintln!();

    let mut fixed = 0usize;
    let mut unchanged = 0usize;
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
        let raw = fs::read_to_string(&manifest)?;
        for f in all_capture_files(&raw) {
            let path = plugin_dir.join(&f);
            match fix_one(&path, &di, dst_sr) {
                Ok(FixResult::Fixed) => {
                    fixed += 1;
                    eprintln!("fix  {}", path.display());
                }
                Ok(FixResult::Unchanged) => unchanged += 1,
                Err(e) => {
                    skipped += 1;
                    eprintln!("skip error  {}: {e:#}", path.display());
                }
            }
        }
    }

    eprintln!();
    eprintln!("qa_fix: fixed={fixed} unchanged={unchanged} skipped={skipped}");
    Ok(())
}

enum FixResult {
    Fixed,
    Unchanged,
}

fn fix_one(path: &Path, probe: &[f32], dst_sr: u32) -> Result<FixResult> {
    let (interleaved, spec) = load_wav_raw(path)?;
    let chans = spec.channels as usize;
    if chans == 0 {
        bail!("zero channels");
    }

    // Deinterleave into per-channel buffers, fix each independently
    // (DC remove + resample). Peak-normalise uses the MAX channel peak
    // so stereo balance is preserved.
    let mut channels: Vec<Vec<f32>> = (0..chans)
        .map(|c| interleaved.iter().skip(c).step_by(chans).copied().collect())
        .collect();
    for ch in channels.iter_mut() {
        *ch = dc_remove(ch);
        if spec.sample_rate != dst_sr {
            *ch = sinc_resample(ch, spec.sample_rate, dst_sr);
        }
    }
    // Compute the scale that brings the worst channel's convolved peak
    // to the target, then apply it uniformly to every channel.
    let max_pre_peak_db = channels
        .iter()
        .map(|ch| {
            let wet =
                loudness_audit::ir::convolve(probe, ch);
            loudness_audit::loudness::peak_dbfs(&wet)
        })
        .filter(|p| p.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    if max_pre_peak_db.is_finite() {
        let scale = 10f32.powf((TARGET_PEAK_DBFS - max_pre_peak_db) / 20.0);
        for ch in channels.iter_mut() {
            for s in ch.iter_mut() {
                *s *= scale;
            }
        }
    } else {
        // No finite peak across any channel — every channel must be
        // empty / silent. Apply DC-remove/resample result and continue;
        // peak-normalise is a no-op for silent buffers.
        for (i, ch) in channels.iter_mut().enumerate() {
            *ch = peak_normalize_for_convolution(probe, ch, TARGET_PEAK_DBFS);
            let _ = i;
        }
    }

    // Re-interleave.
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
    bail!("usage: qa_fix --source <plugins/source path>")
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
