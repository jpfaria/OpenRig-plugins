//! `nam_level_peaks_dbfs` renders captures on several threads; it must
//! give exactly what a serial render gives (issue #143), or the writer
//! and the gate would size gains on corrupted output. Needs the catalogue
//! and the NAM core, so it is opt-in: `cargo test -- --ignored`.

use loudness_audit::level::level_peak_dbfs;
use loudness_audit::nam_run::{nam_level_peaks_dbfs, run_nam};
use loudness_audit::synthetic_di::default_guitar_di;
use std::path::PathBuf;

#[test]
#[ignore]
fn parallel_level_peaks_match_serial_render() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/source/nam/mesa_rectifier_a2/captures");
    let mut models: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "nam"))
        .collect();
    models.sort();
    models.truncate(8);
    assert!(models.len() > 1, "need several captures in {}", dir.display());

    let di = default_guitar_di();
    let parallel = nam_level_peaks_dbfs(&di, &models).unwrap();
    for (model, peak) in models.iter().zip(&parallel) {
        let serial = level_peak_dbfs(&run_nam(&di, model).unwrap());
        assert_eq!(serial, *peak, "{}", model.display());
    }
}
