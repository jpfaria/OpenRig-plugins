//! Shared helpers for the audit binary AND the integration tests.
//!
//! - [`synthetic_di`] — deterministic guitar-like DI signal used by
//!   the LUFS catalog test (real-signal loudness check, NOT the
//!   tautological pink-noise probe roundtrip).
//! - [`catalog`] — minimal plugin walker (lists amp/preamp NAM plugins
//!   under a root, parses just enough YAML to know `type`, first
//!   capture file, and `output_gain_db`).

pub mod catalog;
pub mod synthetic_di;
