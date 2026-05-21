//! Real-signal boost-only catalog test (issue #4).
//!
//! Asserts the contract written into every NAM manifest by the
//! `loudness_audit` binary:
//!
//! 1. **No negative defaults.** `output_gain_db >= 0` for every
//!    plugin — the user never sees a negative number on the slider
//!    when they drop a block into a chain.
//! 2. **Floor at DI loudness.** With the manifest gain applied,
//!    `LUFS_pre >= LUFS_DI − TOLERANCE_LU`. Quiet captures are
//!    boosted up to (≈) the DI's loudness; hot captures stay at
//!    their natural level. The upper bound is intentionally open
//!    — adding an amp is supposed to make the chain louder.
//!
//! Also logs `apply_output_limiter` loss for context — pre-peak above
//! 0 dBFS gets eaten by the tanh limiter; that loss is informational,
//! not asserted.
//!
//! Marked `#[ignore]` because it depends on:
//! - the NAM lib (libNeuralAudioCAPI.dylib) being compiled and
//!   linkable at runtime;
//! - env var `OPENRIG_PLUGINS_NAM_TEST_ROOT` or
//!   `OPENRIG_PLUGINS_ROOT` (uses `<root>/nam` automatically).
//!
//! Run with:
//!     OPENRIG_PLUGINS_NAM_TEST_ROOT=$(pwd)/plugins/source/nam \
//!       cargo test -p loudness-audit --release \
//!       --test catalog_lufs -- --ignored --nocapture

use anyhow::Result;
use std::env;
use std::path::PathBuf;

use nam::processor::{close_model_diag, nam_process, open_model_diag};
use loudness_audit::catalog::list_loudness_normalisable;
use loudness_audit::loudness::{
    apply_output_limiter, db_to_lin, integrated_lufs, peak_dbfs,
};
use loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

/// Tolerance in LU below the DI loudness. Boost-only makeup can't
/// always reach exactly `LUFS_DI` — the peak-headroom clamp may stop
/// short for blocks whose natural peak is already near 0 dBFS, and
/// the integrated-LUFS measurement has a few-tenth-LU floor noise.
/// 3 LU below the DI is the tolerated undershoot before we call it
/// a defect.
const TOLERANCE_LU: f32 = 3.0;

#[test]
#[ignore = "requires NAM lib + plugins root via OPENRIG_PLUGINS_NAM_TEST_ROOT or OPENRIG_PLUGINS_ROOT"]
fn catalog_honors_boost_only_contract() -> Result<()> {
    let root = nam_root_from_env()?;
    let entries = list_loudness_normalisable(&root)?;
    if entries.is_empty() {
        panic!(
            "no amp/preamp/gain_pedal plugins found under {}",
            root.display()
        );
    }

    let di = default_guitar_di();
    let lufs_di = integrated_lufs(&di, DI_SAMPLE_RATE as u32);
    let floor = lufs_di - TOLERANCE_LU;

    println!();
    println!("DI: {} samples @ {} Hz, peak -15 dBFS, LUFS {:+.2}", di.len(), DI_SAMPLE_RATE as u32, lufs_di);
    println!("contract: gain >= 0, post-gain LUFS >= {floor:+.2} (DI − {TOLERANCE_LU:.2} LU)");
    println!();
    println!(
        "{:<48} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "plugin", "lufs", "peak", "lufs_lim", "lim_loss", "gain"
    );

    let mut failures = Vec::new();
    let mut limiter_losses = Vec::new();

    for e in &entries {
        let model_path = e
            .capture_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-utf8 path"))?;
        let model = open_model_diag(model_path)?;
        let mut pre = vec![0.0_f32; di.len()];
        unsafe {
            nam_process(model, &di, &mut pre);
            close_model_diag(model);
        }

        let gain_db = e.output_gain_db.unwrap_or(0.0);
        let g = db_to_lin(gain_db);
        for s in pre.iter_mut() {
            *s *= g;
        }

        let lufs_pre = integrated_lufs(&pre, DI_SAMPLE_RATE as u32);
        let peak_pre = peak_dbfs(&pre);

        // Mirror runtime: same signal post-output_limiter.
        let mut post = pre.clone();
        apply_output_limiter(&mut post);
        let lufs_post = integrated_lufs(&post, DI_SAMPLE_RATE as u32);
        let lim_loss = lufs_pre - lufs_post; // positive = loudness lost

        let marker = if gain_db < 0.0 || lufs_pre < floor { " FAIL" } else { "" };
        println!(
            "{:<48} {:>+8.2}  {:>+8.2}  {:>+8.2}  {:>+8.2}  {:>+8.2}{}",
            e.plugin_id, lufs_pre, peak_pre, lufs_post, lim_loss, gain_db, marker
        );

        limiter_losses.push((e.plugin_id.clone(), lim_loss, peak_pre));
        if gain_db < 0.0 {
            failures.push((e.plugin_id.clone(), format!("negative default gain {gain_db:+.2} dB")));
        } else if lufs_pre < floor {
            failures.push((
                e.plugin_id.clone(),
                format!("post-gain LUFS {lufs_pre:+.2} below floor {floor:+.2}"),
            ));
        }
    }

    println!();
    println!("=== Limiter loss top 10 (perceived loudness eaten by tanh) ===");
    let mut sorted = limiter_losses.clone();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (id, loss, peak) in sorted.iter().take(10) {
        println!("  {id:<46} loss {loss:>5.2} LU   pre-peak {peak:>+6.2} dBFS");
    }
    let avg_loss = limiter_losses.iter().map(|x| x.1).sum::<f32>() / limiter_losses.len() as f32;
    println!("avg limiter loss: {avg_loss:.2} LU");

    println!();
    if !failures.is_empty() {
        for (id, why) in &failures {
            eprintln!("FAIL: {id} — {why}");
        }
        panic!(
            "{} of {} plugins violate the boost-only contract",
            failures.len(),
            entries.len()
        );
    }
    Ok(())
}

fn nam_root_from_env() -> Result<PathBuf> {
    if let Ok(p) = env::var("OPENRIG_PLUGINS_NAM_TEST_ROOT") {
        return Ok(PathBuf::from(p));
    }
    if let Ok(root) = env::var("OPENRIG_PLUGINS_ROOT") {
        return Ok(PathBuf::from(root).join("nam"));
    }
    anyhow::bail!(
        "set OPENRIG_PLUGINS_NAM_TEST_ROOT (= /path/to/plugins/source/nam) \
         or OPENRIG_PLUGINS_ROOT (= /path/to/plugins/source)"
    )
}
