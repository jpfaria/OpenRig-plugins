//! Brick-wall limiter — byte-faithful port of the OpenRig engine block
//! `block-dyn::native_limiter_brickwall` (mono path), pinned at the
//! `develop` commit this repo builds against. The chain QA check runs a
//! representative hot chain through this limiter so it asserts the real
//! AUDIBLE (post-limiter) signal, not the raw pre-limiter sum: A2 hot
//! models legitimately overshoot before the user's chain-end limiter
//! catches them (the cpm 22 / OpenRig#542 scenario).
//!
//! Faithfulness note: this MUST track the engine block sample-for-sample.
//! If the engine limiter changes, re-port it here — never approximate by
//! ear. Constants, the soft-knee target curve, the instant-attack /
//! log-release envelope, and the lookahead peak-hold are copied verbatim
//! from `gain.rs`, `lookahead.rs`, and `mono.rs`.

use std::collections::VecDeque;

// --- gain.rs constants ---------------------------------------------------
const LN_10_OVER_20: f32 = 0.115_129_25;
const LN_9: f32 = 2.197_224_6;
const MIN_LIN: f32 = 1e-10;

pub fn db_to_lin(db: f32) -> f32 {
    (db * LN_10_OVER_20).exp()
}

fn lin_to_db(lin: f32) -> f32 {
    lin.max(MIN_LIN).ln() / LN_10_OVER_20
}

/// Engine `LimiterParams::default()`.
#[derive(Debug, Clone, Copy)]
pub struct LimiterParams {
    pub threshold_db: f32,
    pub ceiling_db: f32,
    pub release_ms: f32,
    pub lookahead_ms: f32,
    pub knee_db: f32,
}

impl Default for LimiterParams {
    fn default() -> Self {
        Self {
            threshold_db: -1.0,
            ceiling_db: -0.1,
            release_ms: 100.0,
            lookahead_ms: 3.0,
            knee_db: 2.0,
        }
    }
}

// --- gain.rs: GainConfig / GainComputer ----------------------------------
struct GainConfig {
    threshold_db: f32,
    knee_db: f32,
    release_coef: f32,
}

impl GainConfig {
    fn new(threshold_db: f32, knee_db: f32, release_ms: f32, sample_rate: f32) -> Self {
        let release_samples = (release_ms * 0.001 * sample_rate).max(1.0);
        let release_coef = 1.0 - (-LN_9 / release_samples).exp();
        Self {
            threshold_db,
            knee_db: knee_db.max(0.0),
            release_coef: release_coef.clamp(0.0, 1.0),
        }
    }
}

/// Target gain reduction (dB, ≤ 0) under the soft-knee brick-wall curve.
fn target_gr_db(peak_db: f32, threshold_db: f32, knee_db: f32) -> f32 {
    let half_knee = knee_db * 0.5;
    let knee_low = threshold_db - half_knee;
    let knee_high = threshold_db + half_knee;

    if peak_db <= knee_low {
        0.0
    } else if peak_db >= knee_high {
        threshold_db - peak_db
    } else if knee_db <= 0.0 {
        (threshold_db - peak_db).min(0.0)
    } else {
        let d = peak_db - knee_low;
        -(d * d) / (2.0 * knee_db)
    }
}

struct GainComputer {
    gr_db: f32,
}

impl GainComputer {
    fn new() -> Self {
        Self { gr_db: 0.0 }
    }

    fn tick(&mut self, peak_lin: f32, cfg: &GainConfig) -> f32 {
        let peak_db = lin_to_db(peak_lin);
        let target = target_gr_db(peak_db, cfg.threshold_db, cfg.knee_db);
        // Instant attack, log release.
        if target < self.gr_db {
            self.gr_db = target;
        } else {
            self.gr_db += (target - self.gr_db) * cfg.release_coef;
        }
        db_to_lin(self.gr_db)
    }
}

// --- lookahead.rs --------------------------------------------------------
struct LookaheadBuffer {
    buffer: Vec<f32>,
    write: usize,
    len: usize,
    peak_deque: VecDeque<(usize, f32)>,
    index: usize,
}

impl LookaheadBuffer {
    fn new(len: usize) -> Self {
        let len = len.max(1);
        Self {
            buffer: vec![0.0; len],
            write: 0,
            len,
            peak_deque: VecDeque::with_capacity(len),
            index: 0,
        }
    }

    fn push(&mut self, input: f32) -> f32 {
        let read = self.write;
        let delayed = self.buffer[read];
        self.buffer[read] = input;
        self.write = (self.write + 1) % self.len;

        let abs = input.abs();
        if self.index >= self.len {
            let expire_index = self.index - self.len;
            while let Some(&(idx, _)) = self.peak_deque.front() {
                if idx <= expire_index {
                    self.peak_deque.pop_front();
                } else {
                    break;
                }
            }
        }
        while let Some(&(_, val)) = self.peak_deque.back() {
            if val <= abs {
                self.peak_deque.pop_back();
            } else {
                break;
            }
        }
        self.peak_deque.push_back((self.index, abs));
        self.index = self.index.wrapping_add(1);

        delayed
    }

    fn peak(&self) -> f32 {
        self.peak_deque.front().map(|&(_, v)| v).unwrap_or(0.0)
    }
}

// --- mono.rs: BrickWallLimiterMono ---------------------------------------
struct BrickWallLimiterMono {
    lookahead: LookaheadBuffer,
    gain: GainComputer,
    cfg: GainConfig,
    ceiling_lin: f32,
}

impl BrickWallLimiterMono {
    fn new(params: LimiterParams, sample_rate: f32) -> Self {
        let sr = sample_rate.max(1.0);
        let lookahead_samples = ((params.lookahead_ms * 0.001) * sr).round().max(1.0) as usize;
        Self {
            lookahead: LookaheadBuffer::new(lookahead_samples),
            gain: GainComputer::new(),
            cfg: GainConfig::new(params.threshold_db, params.knee_db, params.release_ms, sr),
            ceiling_lin: db_to_lin(params.ceiling_db),
        }
    }

    fn process_sample(&mut self, input: f32) -> f32 {
        let delayed = self.lookahead.push(input);
        let peak = self.lookahead.peak();
        let g = self.gain.tick(peak, &self.cfg);
        (delayed * g).clamp(-self.ceiling_lin, self.ceiling_lin)
    }
}

/// Run `signal` through the mono brick-wall limiter with the engine
/// default parameters at `sample_rate`. Returns the post-limiter signal
/// (delayed by the lookahead, as in the engine streaming path).
pub fn limit_default(signal: &[f32], sample_rate: u32) -> Vec<f32> {
    let mut lim = BrickWallLimiterMono::new(LimiterParams::default(), sample_rate as f32);
    signal.iter().map(|&s| lim.process_sample(s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn peak_lin(s: &[f32]) -> f32 {
        s.iter().fold(0.0_f32, |m, &x| m.max(x.abs()))
    }

    /// A signal hotter than the ceiling is brought down to (≈) the ceiling.
    /// Ceiling default −0.1 dBFS → linear ≈ 0.9886. Allow a tiny margin for
    /// the steady-state envelope.
    #[test]
    fn hot_signal_limited_to_ceiling() {
        // +6 dBFS DC-ish block (steady, well past lookahead settling).
        let hot = vec![db_to_lin(6.0); SR as usize];
        let out = limit_default(&hot, SR);
        let ceiling = db_to_lin(-0.1);
        let p = peak_lin(&out);
        assert!(
            p <= ceiling + 1e-3,
            "post-limiter peak {p} exceeds ceiling {ceiling}"
        );
    }

    /// A signal below threshold passes essentially unchanged (unity gain),
    /// only delayed by the lookahead.
    #[test]
    fn quiet_signal_passes_through() {
        let amp = db_to_lin(-12.0);
        let quiet = vec![amp; SR as usize];
        let out = limit_default(&quiet, SR);
        // Steady-state tail is unaffected by the lookahead delay.
        let tail = out[out.len() - 1];
        assert!(
            (tail - amp).abs() < 1e-4,
            "quiet steady-state {tail} should equal input {amp}"
        );
    }
}
