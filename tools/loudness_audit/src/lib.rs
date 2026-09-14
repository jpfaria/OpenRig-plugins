//! Shared helpers for the audit binary AND the integration tests.
//!
//! - [`synthetic_di`] — deterministic guitar-like DI signal used by
//!   the LUFS catalog test (real-signal loudness check, NOT the
//!   tautological pink-noise probe roundtrip).
//! - [`loudness`] — BS.1770 LUFS, peak, runtime-mirror output_limiter.
//! - [`ir`] — IR `.wav` loading + FFT convolution for cab/body
//!   insertion-loss loudness audit.
//! - [`level`] — output-level policy (issue #143): every NAM/IR block's
//!   reference output peaks at the limiter threshold, max without clip.
//! - [`nam_run`] — offline NAM rendering (serial + per-capture parallel
//!   level peaks) shared by the writer and the gate.
//! - [`qa`] — automated QA checks (clip, silence, NaN/Inf, DC,
//!   LUFS band, HF aliasing) used by the `qa_audit` gate (issue #12).
//!   Listening is not a valid verification step in this repo.
//! - [`selector`] — optional `--plugins kind/name[,…]` subset selector
//!   shared by the `qa_audit` and `qa_fix` binaries (issue #28).
//! - [`lv2_uri`] — LV2 `plugin_uri` vs slot binaries vs `data/*.ttl`
//!   consistency check run by `qa_audit` (issue #133).

pub mod ir;
pub mod level;
pub mod limiter;
pub mod loudness;
pub mod lv2_uri;
pub mod nam_run;
pub mod qa;
pub mod selector;
pub mod synthetic_di;
pub mod wav_fix;
