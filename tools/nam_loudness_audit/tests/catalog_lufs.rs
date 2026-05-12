//! Real-signal LUFS catalog test (issue #413, anti-tautology).
//!
//! O teste pink-noise antigo só validava que o audit é internamente
//! consistente: ele recalcula peak com o MESMO probe que o audit
//! usou pra calcular o gain, então sempre passa. Não diz nada sobre
//! loudness perceptual real. Substituído por este.
//!
//! Pipeline mede 2 estágios:
//! 1. PRE-LIMITER: NAM puro + manifest gain. É isso que o audit
//!    optimiza — onde o gain deve aterrissar.
//! 2. POST-LIMITER: mesmo signal passado pelo `output_limiter`
//!    (mirror do `runtime_dsp::output_limiter`). Mostra quanto o
//!    soft tanh come perceived loudness — pra signal com peak
//!    naturalmente dentro do ceiling, perda ≈ 0; pra peaks acima,
//!    perda cresce rápido.
//!
//! O teste falha se o catálogo PRE-LIMITER não estiver dentro de
//! TOLERANCE_LU do TARGET_LUFS, e LOGA a perda do limiter pra que
//! a gente saiba se o audit precisa baixar o target pra dar margem
//! ao limiter.
//!
//! Marcado `#[ignore]` porque depende:
//! - da lib NAM (libNeuralAudioCAPI.dylib) já compilada;
//! - da env var `OPENRIG_PLUGINS_NAM_TEST_ROOT` ou
//!   `OPENRIG_PLUGINS_ROOT` (usa `<root>/nam` automático).
//!
//! Rodar com:
//!     OPENRIG_PLUGINS_NAM_TEST_ROOT=$(pwd)/plugins/source/nam \
//!       cargo test -p nam-loudness-audit --release \
//!       --test catalog_lufs -- --ignored --nocapture

use anyhow::Result;
use std::env;
use std::path::PathBuf;

use nam::processor::{close_model_diag, nam_process, open_model_diag};
use nam_loudness_audit::catalog::list_loudness_normalisable;
use nam_loudness_audit::loudness::{
    apply_output_limiter, db_to_lin, integrated_lufs, peak_dbfs,
};
use nam_loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

/// Loudness target em LUFS integrated. Bate com o TARGET do audit
/// — se mudar lá, mudar aqui também (estão acoplados de propósito).
const TARGET_LUFS: f32 = -10.0;

/// Tolerância em LU. ±3 LU = ~6 dB de spread permitido entre o amp
/// mais alto e o mais baixo. Aceitável: clean amps com crest factor
/// alto não chegam ao mesmo LUFS de saturated por física, sobem
/// até o teto do peak ceiling — diff residual fica nessa banda.
const TOLERANCE_LU: f32 = 3.0;

#[test]
#[ignore = "requires NAM lib + plugins root via OPENRIG_PLUGINS_NAM_TEST_ROOT or OPENRIG_PLUGINS_ROOT"]
fn catalog_meets_lufs_target() -> Result<()> {
    let root = nam_root_from_env()?;
    let entries = list_loudness_normalisable(&root)?;
    if entries.is_empty() {
        panic!(
            "no amp/preamp/gain_pedal plugins found under {}",
            root.display()
        );
    }

    let di = default_guitar_di();

    println!();
    println!("DI: {} samples @ {} Hz, peak -15 dBFS", di.len(), DI_SAMPLE_RATE as u32);
    println!("target: {TARGET_LUFS:+.2} LUFS, tolerance ±{TOLERANCE_LU:.2} LU");
    println!();
    println!(
        "{:<48} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "plugin", "lufs", "peak", "lufs_lim", "lim_loss", "delta"
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

        let delta = lufs_pre - TARGET_LUFS;
        let marker = if delta.abs() > TOLERANCE_LU { " FAIL" } else { "" };
        println!(
            "{:<48} {:>+8.2}  {:>+8.2}  {:>+8.2}  {:>+8.2}  {:>+8.2}{}",
            e.plugin_id, lufs_pre, peak_pre, lufs_post, lim_loss, delta, marker
        );

        limiter_losses.push((e.plugin_id.clone(), lim_loss, peak_pre));
        if delta.abs() > TOLERANCE_LU {
            failures.push((e.plugin_id.clone(), lufs_pre, delta));
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
        for (id, lufs, delta) in &failures {
            eprintln!("FAIL: {id} → {lufs:.2} LUFS ({delta:+.2} LU off target)");
        }
        panic!(
            "{} of {} loudness-normalisable plugins outside LUFS tolerance",
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
