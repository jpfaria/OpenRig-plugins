//! Offline NAM rendering shared by the `loudness_audit` writer and the
//! `qa_audit` gate, so both measure a capture's level the same way.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use nam::processor::{close_model_diag, nam_process, open_model_diag};

use crate::level::level_peak_dbfs;

/// Runs `input` through one `.nam` model and returns the raw output.
pub fn run_nam(input: &[f32], model: &Path) -> Result<Vec<f32>> {
    let p = model
        .to_str()
        .ok_or_else(|| anyhow!("non-utf8 model path: {model:?}"))?;
    let handle = open_model_diag(p).with_context(|| format!("open {p}"))?;
    let mut out = vec![0.0_f32; input.len()];
    unsafe {
        nam_process(handle, input, &mut out);
        close_model_diag(handle);
    }
    Ok(out)
}

/// Level peak ([`level_peak_dbfs`]) of `input` through each model, in
/// `models` order. Spread over the available cores — a NAM plugin can
/// carry dozens of captures and the whole catalogue has thousands. Each
/// worker opens its own model handle; no NAM state crosses threads.
pub fn nam_level_peaks_dbfs(input: &[f32], models: &[PathBuf]) -> Result<Vec<f32>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(models.len().max(1));
    let next = AtomicUsize::new(0);
    let per_worker: Vec<Result<Vec<(usize, f32)>>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                s.spawn(|| -> Result<Vec<(usize, f32)>> {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(model) = models.get(i) else { break };
                        let out = run_nam(input, model)?;
                        mine.push((i, level_peak_dbfs(&out)));
                    }
                    Ok(mine)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("NAM worker panicked"))
            .collect()
    });
    let mut peaks = vec![f32::NEG_INFINITY; models.len()];
    for worker in per_worker {
        for (i, p) in worker? {
            peaks[i] = p;
        }
    }
    Ok(peaks)
}
