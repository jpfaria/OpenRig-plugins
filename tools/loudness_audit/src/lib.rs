//! Shared helpers for the audit binary AND the integration tests.
//!
//! - [`synthetic_di`] — deterministic guitar-like DI signal used by
//!   the LUFS catalog test (real-signal loudness check, NOT the
//!   tautological pink-noise probe roundtrip).
//! - [`catalog`] — minimal plugin walker (lists amp/preamp NAM plugins
//!   under a root, parses just enough YAML to know `type`, first
//!   capture file, and `output_gain_db`).
//! - [`loudness`] — BS.1770 LUFS, peak, runtime-mirror output_limiter.
//! - [`ir`] — IR `.wav` loading + FFT convolution for cab/body
//!   insertion-loss loudness audit.
//! - [`qa`] — automated QA checks (clip, silence, NaN/Inf, DC,
//!   LUFS band, HF aliasing) used by the `qa_audit` gate (issue #12).
//!   Listening is not a valid verification step in this repo.
//! - [`selector`] — optional `--plugins kind/name[,…]` subset selector
//!   shared by the `qa_audit` and `qa_fix` binaries (issue #28).

pub mod catalog;
pub mod ir;
pub mod loudness;
pub mod qa;
pub mod selector;
pub mod synthetic_di;
pub mod wav_fix;
