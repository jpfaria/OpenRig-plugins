//! Deterministic guitar-like DI signal for offline loudness measurement.
//!
//! Por que sintético em vez de WAV real:
//! - Fixture binária no repo é dor de manter (LFS, license, etc).
//! - Determinístico via seed → mesmo signal em qualquer máquina, CI inclusa.
//! - Karplus-Strong + envelope ADSR-ish soa razoavelmente como guitarra
//!   pra fins de medir loudness perceptual: tem ataque, decay natural,
//!   harmônicos por filtro de delay, dinamicidade que LUFS gating
//!   precisa pra contar como "audio significativo" (>= 3s acima do
//!   absolute gate de -70 LUFS).
//!
//! NÃO é o objetivo soar bonito. É o objetivo ser REPRESENTATIVO o
//! suficiente pra que o loudness final do amp + manifest gain bata
//! com o que o ouvido humano percebe na chain real.

const SAMPLE_RATE: f32 = 48_000.0;

/// Sample rate at which the DI is generated. Tests should use this
/// when calling NAM (NAMs themselves are sample-rate agnostic at the
/// model level — the runtime feeds whatever the audio engine has).
pub const DI_SAMPLE_RATE: f32 = SAMPLE_RATE;

/// Default DI used by the LUFS catalog test:
///
/// Three power chords (root + perfect fifth + octave) plucked in
/// sequence, each ringing ~3.8s. Roots: E2 (82.41 Hz), A2 (110 Hz),
/// D3 (146.83 Hz). Total ~12 s.
///
/// Peak normalised to -15 dBFS — typical Scarlett-style line-in
/// guitar peak (real DI lives between -18 and -10 dBFS).
pub fn default_guitar_di() -> Vec<f32> {
    let chords = &[
        (82.41, 0.0, 4.0),
        (110.0, 4.0, 4.0),
        (146.83, 8.0, 4.0),
    ];
    synth_chords(12.0, chords, -15.0)
}

fn synth_chords(total_seconds: f32, chords: &[(f32, f32, f32)], peak_dbfs: f32) -> Vec<f32> {
    let total = (total_seconds * SAMPLE_RATE) as usize;
    let mut buf = vec![0.0_f32; total];

    for &(root_hz, start_s, length_s) in chords {
        let start = (start_s * SAMPLE_RATE) as usize;
        let length = ((length_s * SAMPLE_RATE) as usize).min(total.saturating_sub(start));
        let chord = power_chord(root_hz, length);
        for (i, s) in chord.iter().enumerate() {
            buf[start + i] += *s;
        }
    }

    normalize_peak_dbfs(&mut buf, peak_dbfs);
    buf
}

fn power_chord(root_hz: f32, samples: usize) -> Vec<f32> {
    // Root + perfect-fifth (3/2) + octave (2x). Re-plucked once per
    // second so the chord carries audible energy throughout the
    // 4-second window — Karplus-Strong without re-pluck loses ~40 dB
    // in 1s for low frequencies and the LUFS gate would drop those
    // windows.
    let voicings = [(root_hz, 1.0_f32), (root_hz * 1.5, 0.85), (root_hz * 2.0, 0.7)];
    let pluck_period = SAMPLE_RATE as usize; // 1s
    let mut out = vec![0.0_f32; samples];
    let mut start = 0usize;
    while start < samples {
        let len = (samples - start).min(pluck_period);
        for &(hz, gain) in &voicings {
            let pluck = karplus_strong(hz, len, start as u64);
            for (i, s) in pluck.iter().enumerate() {
                out[start + i] += s * gain;
            }
        }
        start += pluck_period;
    }
    apply_envelope(&mut out);
    out
}

fn karplus_strong(freq_hz: f32, samples: usize, seed_salt: u64) -> Vec<f32> {
    let buf_len = (SAMPLE_RATE / freq_hz).round().max(2.0) as usize;
    let mut delay = vec![0.0_f32; buf_len];

    let seed = 0xC0FFEE_u64
        .wrapping_mul((freq_hz * 100.0) as u64 + 1)
        .wrapping_add(seed_salt.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut rng = XorShift64::new(seed);
    for s in delay.iter_mut() {
        *s = rng.next_f32_signed();
    }

    // Pure Karplus-Strong: average-of-2 lowpass = natural decay from
    // the high-frequency loss of energy each cycle. No extra decay
    // factor (multiplied 0.9965 per sample killed it in <1s).
    let mut out = vec![0.0_f32; samples];
    let mut idx = 0;
    for o in out.iter_mut() {
        let next = (idx + 1) % buf_len;
        *o = delay[idx];
        delay[idx] = (delay[idx] + delay[next]) * 0.5;
        idx = next;
    }
    out
}

/// Quick attack + long natural decay envelope. Karplus-Strong already
/// decays — this just shapes the front so it doesn't open as a click.
fn apply_envelope(buf: &mut [f32]) {
    let attack_samples = (SAMPLE_RATE * 0.005) as usize; // 5ms attack
    for (i, s) in buf.iter_mut().enumerate() {
        if i < attack_samples {
            let r = i as f32 / attack_samples as f32;
            *s *= r;
        }
    }
}

fn normalize_peak_dbfs(buf: &mut [f32], target_dbfs: f32) {
    let peak = buf.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
    if peak == 0.0 {
        return;
    }
    let target_lin = 10.0_f32.powf(target_dbfs / 20.0);
    let scale = target_lin / peak;
    for s in buf.iter_mut() {
        *s *= scale;
    }
}

struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xDEAD_BEEF } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_f32_signed(&mut self) -> f32 {
        (self.next_u64() as f64 / u64::MAX as f64) as f32 * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_di_is_twelve_seconds_at_48k() {
        let di = default_guitar_di();
        assert_eq!(di.len(), (12.0 * SAMPLE_RATE) as usize);
    }

    #[test]
    fn default_di_peak_is_minus_15_dbfs() {
        let di = default_guitar_di();
        let peak = di.iter().fold(0.0_f32, |acc, s| acc.max(s.abs()));
        let peak_dbfs = 20.0 * peak.log10();
        assert!(
            (peak_dbfs - (-15.0)).abs() < 0.01,
            "peak was {peak_dbfs:.4} dBFS"
        );
    }

    #[test]
    fn default_di_is_deterministic() {
        let a = default_guitar_di();
        let b = default_guitar_di();
        assert_eq!(a, b, "synth must be deterministic for CI repeatability");
    }

    #[test]
    fn default_di_has_audio_throughout() {
        let di = default_guitar_di();
        // Each 3-second chord region should have non-trivial RMS so
        // LUFS gating doesn't drop it. Check 1s windows around 1s,
        // 5s, 9s.
        for &center_s in &[1.0_f32, 5.0, 9.0] {
            let start = (center_s * SAMPLE_RATE) as usize;
            let end = ((center_s + 0.5) * SAMPLE_RATE) as usize;
            let win = &di[start..end];
            let rms = (win.iter().map(|s| s * s).sum::<f32>() / win.len() as f32).sqrt();
            assert!(
                rms > 0.01,
                "window centred on {center_s}s has RMS {rms:.4} — too quiet"
            );
        }
    }
}
