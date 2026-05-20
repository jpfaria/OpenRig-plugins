//! `qa_audit` — automated quality gate for plugin outputs (issue #12).
//!
//! Runs every loudness-normalisable plugin (NAM amp/preamp/gain_pedal
//! and IR cab/body) through a deterministic probe DI and asserts every
//! check in `loudness_audit::qa`. Exit non-zero on any failure;
//! `pack_plugins` aborts the release on a non-zero exit.
//!
//! Validation by ear is not accepted in this repo. Every failure mode
//! that ships is encoded here as a threshold.
//!
//! Scaffold only — plugin/chain orchestration lands in subsequent
//! tasks (QA3–QA5 of issue #12).

fn main() {
    eprintln!("qa_audit: scaffold — orchestration pending (QA3–QA5)");
    std::process::exit(0);
}
