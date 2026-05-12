//! Real-signal LUFS catalog test (issue #413, anti-tautology).
//!
//! O teste pink-noise antigo (`catalog_levelling.rs`) é tautológico:
//! audit calcula gain pra fazer peak bater, teste mede peak com mesmo
//! probe e gain. Sempre passa. NÃO prova nada sobre o que o ouvido
//! humano percebe na chain real.
//!
//! Este teste:
//! 1. Gera DI sintético (Karplus-Strong power chords) com peak típico
//!    de guitarra real (-15 dBFS).
//! 2. Pra cada amp/preamp do catálogo: passa pelo NAM, soma o
//!    `output_gain_db` do manifest.
//! 3. Mede LUFS integrated (BS.1770, perceptual).
//! 4. Falha se algum amp sair além de TOLERANCE_LU do TARGET_LUFS.
//!
//! Marcado `#[ignore]` porque depende:
//! - da lib NAM (libNeuralAudioCAPI.dylib) já compilada;
//! - da env var `OPENRIG_PLUGINS_NAM_TEST_ROOT` (ou
//!   `OPENRIG_PLUGINS_ROOT` — usa `<root>/nam` automático).
//!
//! Rodar com:
//!     OPENRIG_PLUGINS_NAM_TEST_ROOT=$(pwd)/plugins/source/nam \
//!       cargo test -p nam-loudness-audit --release \
//!       --test catalog_lufs -- --ignored --nocapture

use anyhow::Result;
use std::env;
use std::path::PathBuf;

use nam::processor::{close_model_diag, nam_process, open_model_diag};
use nam_loudness_audit::catalog::list_amp_preamp;
use nam_loudness_audit::synthetic_di::{default_guitar_di, DI_SAMPLE_RATE};

/// Loudness target em LUFS integrated. -14 LUFS é o standard de
/// streaming (Spotify / Apple Music) e funciona como referência audível
/// neutra: alto o suficiente pra ser percebido em fone modesto, com
/// headroom pra dinâmica.
const TARGET_LUFS: f32 = -14.0;

/// Tolerância em LU. ±3 LU = ~6 dB de spread permitido entre o amp
/// mais alto e o mais baixo. Aceitável: clean jazz amp soa um pouco
/// mais quieto que high-gain saturado por física do modelo, mas a
/// brecha não pode ser absurda.
const TOLERANCE_LU: f32 = 3.0;

#[test]
#[ignore = "requires NAM lib + plugins root via OPENRIG_PLUGINS_NAM_TEST_ROOT or OPENRIG_PLUGINS_ROOT"]
fn catalog_meets_lufs_target() -> Result<()> {
    let root = nam_root_from_env()?;
    let entries = list_amp_preamp(&root)?;
    if entries.is_empty() {
        panic!("no amp/preamp plugins found under {}", root.display());
    }

    let di = default_guitar_di();

    println!();
    println!("DI: {} samples @ {} Hz, peak -15 dBFS", di.len(), DI_SAMPLE_RATE as u32);
    println!("target: {TARGET_LUFS:.2} LUFS, tolerance ±{TOLERANCE_LU:.2} LU");
    println!();
    println!("{:<48} {:>10} {:>10}", "plugin", "lufs", "delta");

    let mut failures = Vec::new();
    for e in &entries {
        let model_path = e
            .capture_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-utf8 path"))?;
        let model = open_model_diag(model_path)?;
        let mut out = vec![0.0_f32; di.len()];
        unsafe {
            nam_process(model, &di, &mut out);
            close_model_diag(model);
        }

        let gain_db = e.output_gain_db.unwrap_or(0.0);
        let g = db_to_lin(gain_db);
        for s in out.iter_mut() {
            *s *= g;
        }

        let lufs = measure_integrated_lufs(&out);
        let delta = lufs - TARGET_LUFS;
        let marker = if delta.abs() > TOLERANCE_LU { " FAIL" } else { "" };
        println!("{:<48} {:>9.2}  {:>+9.2}{}", e.plugin_id, lufs, delta, marker);
        if delta.abs() > TOLERANCE_LU {
            failures.push((e.plugin_id.clone(), lufs, delta));
        }
    }

    println!();
    if !failures.is_empty() {
        for (id, lufs, delta) in &failures {
            eprintln!("FAIL: {id} → {lufs:.2} LUFS ({delta:+.2} LU off target)");
        }
        panic!(
            "{} of {} amp/preamp plugins outside LUFS tolerance",
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

fn db_to_lin(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn measure_integrated_lufs(samples: &[f32]) -> f32 {
    let mut meter = bs1770::ChannelLoudnessMeter::new(DI_SAMPLE_RATE as u32);
    meter.push(samples.iter().copied());
    let windows = meter.into_100ms_windows();
    let gated = bs1770::gated_mean(windows.as_ref());
    gated.loudness_lkfs()
}
