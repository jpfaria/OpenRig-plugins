# Cab/Body IR Loudness Compensation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Measure each cab/body IR's insertion loss and persist a per-capture `output_gain_db` in the manifest so passing the signal through an IR block never loses volume.

**Architecture:** Extend the `nam_loudness_audit` tool with an IR path: load each capture's `.wav`, convolve the existing deterministic synthetic DI through it, measure integrated LUFS in vs out, write `output_gain_db = max(0, LUFS_in − LUFS_out)` (boost-only, peak-clamped) **per capture entry**. Cab/body is net-new (no collision with the in-flight OpenRig#491 top-level amp read). Amp/preamp/gain_pedal migration to the same per-capture location is the final task, gated on OpenRig#491 confirming a per-capture read.

**Tech Stack:** Rust, `bs1770` (already), `hound` (WAV I/O, new), `realfft` (FFT overlap-add convolution, new). LUFS of a linear convolution is convolver-implementation-independent, so a self-contained FFT convolver is loudness-correct and removes any external-API guesswork.

**Cross-repo gate:** OpenRig#491 must resolve `output_gain_db` per selected capture. Tasks 1–7 (producer + manifests) are independent of #491. The end-to-end "loudness-transparent in OpenRig" acceptance criterion and Task 8 (amp migration) are BLOCKED on #491 confirmation — see issue #8.

**Facts established (verified, do not re-derive):**
- `DI_SAMPLE_RATE` = `48_000.0` (`tools/nam_loudness_audit/src/synthetic_di.rs:16,21`).
- IR `.wav` are mono; most 48000 Hz, a minority 44100 Hz; 24-bit PCM (`sampwidth 3`).
- Existing per-manifest writer `upsert_output_gain_db` anchors on top-level `type:` — NOT reusable per-capture; a new writer is required.
- Capture grid shape:
  ```
  captures:
  - values:
      mic: r121
      position: cap
    file: ir/foo.wav
  - values:
      flavor: standard
    file: ir/bar.wav
  ```

---

### Task 1: Add deps + WAV loader

**Files:**
- Modify: `tools/nam_loudness_audit/Cargo.toml`
- Create: `tools/nam_loudness_audit/src/ir.rs`
- Modify: `tools/nam_loudness_audit/src/lib.rs`

- [ ] **Step 1: Add dependencies**

In `tools/nam_loudness_audit/Cargo.toml`, under `[dependencies]` add:

```toml
hound = "3"
realfft = "3"
```

- [ ] **Step 2: Register module**

In `tools/nam_loudness_audit/src/lib.rs`, add after `pub mod catalog;`:

```rust
pub mod ir;
```

- [ ] **Step 3: Write failing test for `load_wav_ir`**

Create `tools/nam_loudness_audit/src/ir.rs`:

```rust
//! IR (.wav) loading + FFT convolution for cab/body loudness audit.
//! LUFS of a linear convolution is implementation-independent, so this
//! self-contained convolver is loudness-correct without depending on
//! the runtime IR crate.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Loads a mono IR `.wav` as `f32` samples normalised to [-1, 1],
/// resampled to 48 kHz if needed. Stereo files are downmixed (mean).
pub fn load_wav_ir(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let spec = reader.spec();
    let chans = spec.channels as usize;
    if chans == 0 {
        bail!("{}: zero channels", path.display());
    }
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.samples::<f32>().collect::<Result<_, _>>()?
        }
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / max))
                .collect::<Result<_, _>>()?
        }
    };
    // Interleaved -> mono mean.
    let mono: Vec<f32> = raw
        .chunks(chans)
        .map(|f| f.iter().sum::<f32>() / chans as f32)
        .collect();
    Ok(resample_linear(&mono, spec.sample_rate, 48_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_known_wav_mono_normalised() {
        // Build a 4-sample 48k mono i16 wav in a temp file.
        let dir = std::env::temp_dir().join("irtest_load");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&p, spec).unwrap();
        for v in [0i16, 16384, -16384, 32767] {
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let got = load_wav_ir(&p).unwrap();
        assert_eq!(got.len(), 4);
        assert!((got[1] - 0.5).abs() < 1e-3);
        assert!((got[3] - 1.0).abs() < 1e-3);
    }
}
```

Add a placeholder so it compiles for the failing run:

```rust
pub fn resample_linear(_x: &[f32], _from: u32, _to: u32) -> Vec<f32> {
    unimplemented!()
}
```

- [ ] **Step 4: Run test, expect fail**

Run: `cargo test -p nam-loudness-audit ir::tests::loads_known_wav_mono_normalised`
Expected: FAIL (`not implemented`).

- [ ] **Step 5: Commit**

```bash
git add tools/nam_loudness_audit/Cargo.toml tools/nam_loudness_audit/src/lib.rs tools/nam_loudness_audit/src/ir.rs
git commit -m "feat(audit): wav IR loader scaffold for cab/body (#8)"
```

---

### Task 2: Linear resampler

**Files:**
- Modify: `tools/nam_loudness_audit/src/ir.rs`

- [ ] **Step 1: Write failing test**

Append to `mod tests` in `ir.rs`:

```rust
#[test]
fn resample_44k_to_48k_scales_length_and_keeps_endpoints() {
    let x: Vec<f32> = (0..441).map(|i| i as f32 / 440.0).collect();
    let y = resample_linear(&x, 44_100, 48_000);
    // length ~ ceil(441 * 48000/44100) = 480
    assert!((y.len() as i32 - 480).abs() <= 1, "len was {}", y.len());
    assert!((y[0] - 0.0).abs() < 1e-4);
    assert!((*y.last().unwrap() - 1.0).abs() < 2e-2);
}

#[test]
fn resample_noop_when_rates_equal() {
    let x = vec![0.1, 0.2, 0.3];
    assert_eq!(resample_linear(&x, 48_000, 48_000), x);
}
```

- [ ] **Step 2: Run, expect fail**

Run: `cargo test -p nam-loudness-audit ir::tests::resample`
Expected: FAIL (`not implemented`).

- [ ] **Step 3: Implement**

Replace the placeholder `resample_linear` with:

```rust
/// Linear-interpolation resample. Adequate for an integrated-LUFS
/// measurement of a short IR (tone error is irrelevant to the loudness
/// number; correctness over fidelity, no silent skip on 44.1k IRs).
pub fn resample_linear(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() {
        return x.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let out_len = (x.len() as f64 * ratio).round() as usize;
    let mut y = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src = i as f64 / ratio;
        let i0 = src.floor() as usize;
        let frac = (src - i0 as f64) as f32;
        let a = x.get(i0).copied().unwrap_or(0.0);
        let b = x.get(i0 + 1).copied().unwrap_or(a);
        y.push(a + (b - a) * frac);
    }
    y
}
```

- [ ] **Step 4: Run, expect pass**

Run: `cargo test -p nam-loudness-audit ir::tests::`
Expected: PASS (resample + load tests).

- [ ] **Step 5: Commit**

```bash
git add tools/nam_loudness_audit/src/ir.rs
git commit -m "feat(audit): linear resampler for non-48k IRs (#8)"
```

---

### Task 3: FFT overlap-add convolution

**Files:**
- Modify: `tools/nam_loudness_audit/src/ir.rs`

- [ ] **Step 1: Write failing test**

Append to `mod tests`:

```rust
#[test]
fn delta_ir_is_identity() {
    let sig = vec![0.1, -0.4, 0.7, 0.2, -0.9];
    let ir = vec![1.0_f32];
    let y = convolve(&sig, &ir);
    for (a, b) in sig.iter().zip(y.iter()) {
        assert!((a - b).abs() < 1e-5, "{a} vs {b}");
    }
}

#[test]
fn scaled_delta_scales_signal() {
    let sig = vec![0.1, -0.4, 0.7, 0.2, -0.9];
    let ir = vec![0.5_f32]; // -6 dB
    let y = convolve(&sig, &ir);
    for (a, b) in sig.iter().zip(y.iter()) {
        assert!((a * 0.5 - b).abs() < 1e-5);
    }
}

#[test]
fn matches_naive_convolution() {
    let sig: Vec<f32> = (0..200).map(|i| (i as f32 * 0.3).sin()).collect();
    let ir: Vec<f32> = (0..37).map(|i| (i as f32 * 0.11).cos() * 0.2).collect();
    let fast = convolve(&sig, &ir);
    let mut naive = vec![0.0_f32; sig.len() + ir.len() - 1];
    for (i, s) in sig.iter().enumerate() {
        for (j, h) in ir.iter().enumerate() {
            naive[i + j] += s * h;
        }
    }
    assert_eq!(fast.len(), naive.len());
    for (a, b) in fast.iter().zip(naive.iter()) {
        assert!((a - b).abs() < 1e-3, "{a} vs {b}");
    }
}
```

- [ ] **Step 2: Run, expect fail**

Run: `cargo test -p nam-loudness-audit ir::tests::matches_naive_convolution`
Expected: FAIL (`convolve` not found — compile error).

- [ ] **Step 3: Implement**

Add to `ir.rs` (above `mod tests`):

```rust
use realfft::RealFftPlanner;

/// Full linear convolution via a single zero-padded FFT. IR + DI are
/// short enough (DI a few sec @ 48k, IR ≤ a few k taps) for one
/// transform; output length = sig + ir − 1.
pub fn convolve(sig: &[f32], ir: &[f32]) -> Vec<f32> {
    if sig.is_empty() || ir.is_empty() {
        return Vec::new();
    }
    let n_lin = sig.len() + ir.len() - 1;
    let n = n_lin.next_power_of_two();
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let ifft = planner.plan_fft_inverse(n);

    let mut a = fft.make_input_vec();
    let mut b = fft.make_input_vec();
    a[..sig.len()].copy_from_slice(sig);
    b[..ir.len()].copy_from_slice(ir);

    let mut sa = fft.make_output_vec();
    let mut sb = fft.make_output_vec();
    fft.process(&mut a, &mut sa).unwrap();
    fft.process(&mut b, &mut sb).unwrap();

    for (x, y) in sa.iter_mut().zip(sb.iter()) {
        *x *= *y;
    }
    let mut out = ifft.make_output_vec();
    ifft.process(&mut sa, &mut out).unwrap();

    let scale = 1.0 / n as f32;
    out.truncate(n_lin);
    out.iter_mut().for_each(|v| *v *= scale);
    out
}
```

- [ ] **Step 4: Run, expect pass**

Run: `cargo test -p nam-loudness-audit ir::tests::`
Expected: PASS (all ir tests).

- [ ] **Step 5: Commit**

```bash
git add tools/nam_loudness_audit/src/ir.rs
git commit -m "feat(audit): fft linear convolution for IR loudness (#8)"
```

---

### Task 4: Enumerate all captures (not just first)

**Files:**
- Modify: `tools/nam_loudness_audit/src/main.rs`

- [ ] **Step 1: Write failing test**

In `main.rs` `mod tests`, add:

```rust
#[test]
fn lists_all_capture_files_in_order() {
    let yaml = "captures:\n- values:\n    mic: a\n  file: ir/one.wav\n- values:\n    mic: b\n  file: ir/two.wav\n";
    assert_eq!(
        all_capture_files(yaml),
        vec!["ir/one.wav".to_string(), "ir/two.wav".to_string()]
    );
}
```

- [ ] **Step 2: Run, expect fail**

Run: `cargo test -p nam-loudness-audit tests::lists_all_capture_files_in_order`
Expected: FAIL (`all_capture_files` not found).

- [ ] **Step 3: Implement**

Add to `main.rs` (near `first_capture_file`):

```rust
/// Every `file:` under `captures:`, in document order.
fn all_capture_files(yaml: &str) -> Vec<String> {
    let mut in_captures = false;
    let mut files = Vec::new();
    for line in yaml.lines() {
        let trimmed = line.trim_start();
        if !line.starts_with(char::is_whitespace) && trimmed.starts_with("captures:") {
            in_captures = true;
            continue;
        }
        if in_captures
            && !line.starts_with(char::is_whitespace)
            && trimmed.ends_with(':')
        {
            break; // next top-level key ends the captures block
        }
        if !in_captures {
            continue;
        }
        let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        if let Some(rest) = after_dash.strip_prefix("file:") {
            files.push(rest.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    files
}
```

- [ ] **Step 4: Run, expect pass**

Run: `cargo test -p nam-loudness-audit tests::lists_all_capture_files_in_order`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/nam_loudness_audit/src/main.rs
git commit -m "feat(audit): enumerate every capture file (#8)"
```

---

### Task 5: Per-capture `output_gain_db` writer

**Files:**
- Modify: `tools/nam_loudness_audit/src/main.rs`

- [ ] **Step 1: Write failing test**

In `main.rs` `mod tests`, add:

```rust
#[test]
fn inserts_gain_per_capture_after_file_line() {
    let yaml = "type: cab\ncaptures:\n- values:\n    mic: a\n  file: ir/one.wav\n- values:\n    mic: b\n  file: ir/two.wav\n";
    let gains = vec![
        ("ir/one.wav".to_string(), 4.0_f32),
        ("ir/two.wav".to_string(), 9.5_f32),
    ];
    let out = upsert_capture_output_gain_db(yaml, &gains);
    assert!(out.contains("  file: ir/one.wav\n  output_gain_db: 4.0000000"));
    assert!(out.contains("  file: ir/two.wav\n  output_gain_db: 9.5000000"));
    assert!(out.starts_with("type: cab\n"));
}

#[test]
fn replaces_existing_per_capture_gain_in_place() {
    let yaml = "captures:\n- file: ir/one.wav\n  output_gain_db: 1.0000000\n";
    let gains = vec![("ir/one.wav".to_string(), 7.0_f32)];
    let out = upsert_capture_output_gain_db(yaml, &gains);
    assert!(out.contains("output_gain_db: 7.0000000"));
    assert!(!out.contains("1.0000000"));
}
```

- [ ] **Step 2: Run, expect fail**

Run: `cargo test -p nam-loudness-audit tests::inserts_gain_per_capture_after_file_line`
Expected: FAIL (`upsert_capture_output_gain_db` not found).

- [ ] **Step 3: Implement**

Add to `main.rs`:

```rust
use std::collections::HashMap;

/// Inserts/replaces `output_gain_db:` as a sibling of each capture's
/// `file:` line, at the same indentation, preserving all other YAML
/// bytes. Keyed by the `file:` value so order/structure is irrelevant.
fn upsert_capture_output_gain_db(yaml: &str, gains: &[(String, f32)]) -> String {
    let map: HashMap<&str, f32> =
        gains.iter().map(|(f, g)| (f.as_str(), *g)).collect();
    let trailing_newline = yaml.ends_with('\n');
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + gains.len());

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        let after_dash = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        out.push(line.to_string());

        if let Some(rest) = after_dash.strip_prefix("file:") {
            let file = rest.trim().trim_matches('"').trim_matches('\'');
            if let Some(&g) = map.get(file) {
                let indent_len = line.len() - trimmed.len();
                let indent = &line[..indent_len];
                let gain_line = format!("{indent}output_gain_db: {g:.7}");
                // Drop an existing sibling output_gain_db on the next line.
                if let Some(next) = lines.get(i + 1) {
                    if next.trim_start().starts_with("output_gain_db:")
                        && (next.len() - next.trim_start().len()) == indent_len
                    {
                        i += 1; // skip stale line
                    }
                }
                out.push(gain_line);
            }
        }
        i += 1;
    }
    let body = out.join("\n");
    if trailing_newline {
        format!("{body}\n")
    } else {
        body
    }
}
```

- [ ] **Step 4: Run, expect pass**

Run: `cargo test -p nam-loudness-audit tests::`
Expected: PASS (both new tests + existing).

- [ ] **Step 5: Commit**

```bash
git add tools/nam_loudness_audit/src/main.rs
git commit -m "feat(audit): per-capture output_gain_db writer (#8)"
```

---

### Task 6: IR audit path (insertion-loss measurement)

**Files:**
- Modify: `tools/nam_loudness_audit/src/main.rs`

- [ ] **Step 1: Write failing test for the gain math**

In `main.rs` `mod tests`, add:

```rust
#[test]
fn insertion_loss_is_boost_only_and_peak_clamped() {
    use nam_loudness_audit::ir::convolve;
    use nam_loudness_audit::loudness::integrated_lufs;
    let di = nam_loudness_audit::synthetic_di::default_guitar_di();

    // −6 dB IR (scaled delta): expected makeup ≈ +6 dB, > 0.
    let ir_atten = vec![0.5_f32];
    let g = ir_capture_gain_db(&di, &ir_atten);
    assert!(g > 4.0 && g < 8.0, "atten makeup was {g}");

    // +6 dB IR: never attenuate -> clamped to 0.
    let ir_boost = vec![2.0_f32];
    let g2 = ir_capture_gain_db(&di, &ir_boost);
    assert_eq!(g2, 0.0, "must be boost-only, was {g2}");

    let _ = (convolve(&di, &ir_atten), integrated_lufs(&di, 48_000));
}
```

- [ ] **Step 2: Run, expect fail**

Run: `cargo test -p nam-loudness-audit tests::insertion_loss_is_boost_only_and_peak_clamped`
Expected: FAIL (`ir_capture_gain_db` not found).

- [ ] **Step 3: Implement the measurement + wire the IR branch**

Add to `main.rs`:

```rust
use nam_loudness_audit::ir::{convolve, load_wav_ir};

/// Makeup for one IR so the block does not lose volume.
/// `max(0, LUFS_in − LUFS_out)`, clamped by the same peak ceiling and
/// MAX as the NAM path. Boost-only: an IR that adds level gets 0.
fn ir_capture_gain_db(di: &[f32], ir: &[f32]) -> f32 {
    let wet = convolve(di, ir);
    let lufs_in = integrated_lufs(di, DI_SAMPLE_RATE as u32);
    let lufs_out = integrated_lufs(&wet, DI_SAMPLE_RATE as u32);
    let peak_out = peak_dbfs(&wet);
    let want = lufs_in - lufs_out;
    let allowed_by_peak = PEAK_CEILING_DBFS - peak_out;
    want.min(allowed_by_peak).clamp(MIN_GAIN_DB, MAX_GAIN_DB)
}

fn audit_ir_plugin(
    plugin_dir: &Path,
    manifest_path: &Path,
    raw: &str,
    di: &[f32],
) -> Result<AuditReport> {
    let files = all_capture_files(raw);
    if files.is_empty() {
        bail!("no `captures:[].file` entry in manifest");
    }
    let mut gains: Vec<(String, f32)> = Vec::with_capacity(files.len());
    let mut sum = 0.0_f32;
    for f in &files {
        let ir = load_wav_ir(&plugin_dir.join(f))
            .with_context(|| format!("load IR {f}"))?;
        let g = ir_capture_gain_db(di, &ir);
        sum += g;
        gains.push((f.clone(), g));
    }
    let updated = upsert_capture_output_gain_db(raw, &gains);
    fs::write(manifest_path, updated)
        .with_context(|| format!("write {}", manifest_path.display()))?;
    let mean = sum / files.len() as f32;
    Ok(AuditReport {
        measured_lufs: f32::NAN,
        measured_peak_dbfs: f32::NAN,
        want_for_lufs_db: mean,
        applied_gain_db: mean,
    })
}
```

Replace `is_loudness_normalisable`:

```rust
fn is_loudness_normalisable(block_type: &str) -> bool {
    matches!(
        block_type,
        "amp" | "preamp" | "gain_pedal" | "cab" | "body"
    )
}
```

In `audit_plugin`, after computing `block_type` and the
`is_loudness_normalisable` check, branch by backend before the NAM load:

```rust
    if matches!(block_type.as_str(), "cab" | "body") {
        return audit_ir_plugin(plugin_dir, manifest_path, &raw, di);
    }
```

(Place it immediately after the `if !is_loudness_normalisable(...)` bail
and before `let first_capture = first_capture_file(&raw)`.)

- [ ] **Step 4: Update the stale unit test**

In `loudness_normalisable_includes_amp_preamp_and_gain_pedal`, move
`"cab"` and `"body"` out of the skip list:

```rust
for ok in ["amp", "preamp", "gain_pedal", "cab", "body"] {
    assert!(is_loudness_normalisable(ok));
}
for skip in ["reverb", "delay", "filter", "utility"] {
    assert!(!is_loudness_normalisable(skip));
}
```

- [ ] **Step 5: Run, expect pass**

Run: `cargo test -p nam-loudness-audit`
Expected: PASS (all tests, including the updated normalisable test).

- [ ] **Step 6: Commit**

```bash
git add tools/nam_loudness_audit/src/main.rs
git commit -m "feat(audit): IR insertion-loss path for cab/body (#8)"
```

---

### Task 7: Run audit over cab/body, gate, docs, commit manifests

**Files:**
- Modify: `plugins/source/ir/**/manifest.yaml` (generated)
- Modify: `tools/nam_loudness_audit/src/main.rs` (doc comment)
- Modify: `.claude/skills/openrig-code-quality/SKILL.md` if methodology changed
- Check: `CLAUDE.md`, `docs/**`, `README*.md` for stale "only amp/preamp" wording

- [ ] **Step 1: Update the tool doc comment**

In `main.rs` top doc comment, replace the NAM-only framing and the
"Cab/body/eq ficam de fora" sentence in `is_loudness_normalisable`'s
doc with English text stating cab/body are now measured per-capture via
IR convolution of the synthetic DI (insertion-loss makeup, boost-only).

- [ ] **Step 2: Grep for stale wording and fix in this commit**

Run:
```bash
grep -rn "only amp/preamp\|amp/preamp\|cab/body.*fora\|spectral shaper" \
  CLAUDE.md README*.md docs/ .claude/skills/ tools/nam_loudness_audit/src/ 2>/dev/null
```
Fix every hit that claims cab/body carry no loudness signature / are excluded.

- [ ] **Step 3: Run the audit over the IR catalogue**

Run:
```bash
cargo run --release -p nam-loudness-audit -- plugins/source/ir
```
Expected: per-plugin lines printed, `audited <N> plugins, skipped 0` (cab+body = 134).

- [ ] **Step 4: Sanity-check the manifest deltas**

Run:
```bash
git diff --stat plugins/source/ir | tail -1
grep -rL "output_gain_db" plugins/source/ir --include=manifest.yaml | wc -l
```
Expected: 134 manifests changed; `0` IR manifests without the field. Spot-check one diff: `output_gain_db` appears once per capture, indented as a `file:` sibling, no other structural change.

- [ ] **Step 5: Mandatory gate**

Run:
```bash
cargo run --release --bin pack_plugins
```
Expected: exit 0, `0 failed`. If red, root-cause — never silence.

- [ ] **Step 6: Commit (two concerns, two commits)**

```bash
git add tools/nam_loudness_audit/src/main.rs CLAUDE.md README*.md docs/ .claude/skills/
git commit -m "docs: cab/body now loudness-normalised via IR audit (#8)"
git add plugins/source/ir
git commit -m "chore(loudness): per-capture output_gain_db for cab/body IRs (#8)"
```

- [ ] **Step 7: Log on the issue**

Comment on jpfaria/OpenRig-plugins#8: both commit hashes, files-touched summary, `pack_plugins` result (exit 0 / `0 failed`), and the audit `audited N skipped 0` line.

---

### Task 8: Amp/preamp/gain_pedal migration to per-capture — GATED on OpenRig#491

**Do not start until OpenRig#491 confirms the engine resolves `output_gain_db` per selected capture.**

**Files:**
- Modify: `tools/nam_loudness_audit/src/main.rs`
- Modify: `plugins/source/nam/**/manifest.yaml` (generated)

- [ ] **Step 1: Confirm the gate**

Verify the OpenRig#491 coordinating comment thread states per-capture read is implemented/scheduled. If not, STOP and report — do not migrate.

- [ ] **Step 2: Route NAM path through the per-capture writer**

In `audit_plugin`, the NAM branch currently calls
`upsert_output_gain_db(&raw, applied)` (top-level). Change it to write
the single NAM capture's gain via `upsert_capture_output_gain_db`,
keyed by the NAM `first_capture_file`, and remove the top-level
`output_gain_db` if present (NAM plugins have one capture, so the
numeric value is unchanged — only the field location moves).

```rust
let file = first_capture_file(&raw)
    .ok_or_else(|| anyhow!("no `captures:[].file` entry"))?;
let updated = strip_top_level_output_gain_db(&raw);
let updated = upsert_capture_output_gain_db(&updated, &[(file, applied)]);
fs::write(manifest_path, updated)?;
```

Add `strip_top_level_output_gain_db` (removes a column-0
`output_gain_db:` line) with a unit test:

```rust
fn strip_top_level_output_gain_db(yaml: &str) -> String {
    let trailing = yaml.ends_with('\n');
    let body = yaml
        .lines()
        .filter(|l| !l.starts_with("output_gain_db:"))
        .collect::<Vec<_>>()
        .join("\n");
    if trailing { format!("{body}\n") } else { body }
}

#[test]
fn strips_only_top_level_gain() {
    let y = "id: x\noutput_gain_db: 3.0\ntype: amp\ncaptures:\n- file: c.nam\n  output_gain_db: 3.0\n";
    let out = strip_top_level_output_gain_db(y);
    assert!(!out.contains("\noutput_gain_db: 3.0\ntype"));
    assert!(out.contains("  output_gain_db: 3.0"));
}
```

- [ ] **Step 3: Tests + re-audit + gate**

```bash
cargo test -p nam-loudness-audit
cargo run --release -p nam-loudness-audit -- plugins/source/nam
cargo run --release --bin pack_plugins
```
Expected: tests PASS; 274 NAM manifests rewritten (field moved into the capture, value unchanged); gate exit 0.

- [ ] **Step 4: Commit + log**

```bash
git add tools/nam_loudness_audit/src/main.rs
git commit -m "refactor(audit): single per-capture output_gain_db location (#8)"
git add plugins/source/nam
git commit -m "chore(loudness): migrate NAM output_gain_db to per-capture (#8)"
```
Comment hashes + gate result on issue #8. Final summary on #8.

---

## Self-Review

**Spec coverage:**
- Per-capture compensation → Tasks 5, 6 (cab/body), Task 8 (amp migration to same location).
- IR-by-IR measurement with existing synthetic DI → Task 6 (`ir_capture_gain_db` uses `default_guitar_di`).
- `max(0, LUFS_in − LUFS_out)`, boost-only, peak-clamped → Task 6 (`ir_capture_gain_db`, MIN/PEAK/MAX reused).
- Tool extension (`is_loudness_normalisable` + IR path) → Task 6.
- Single canonical field location → Task 5 writer + Task 8 NAM migration.
- Engine coupling (OpenRig#491) → explicit gate on Task 8 and on the e2e acceptance criterion (issue #8).
- Docs-in-sync → Task 7 steps 1–2 (doc comment, grep stale wording), same commit.
- Gate before push → Task 7 step 5, Task 8 step 3.
- 44.1k IR handling → Task 2 (resampler; no silent skip).

**Placeholder scan:** No TBD/TODO. Every code step shows complete code. The only deferred item (sample-rate) is resolved in Task 2, not deferred.

**Type consistency:** `load_wav_ir`, `resample_linear`, `convolve`, `all_capture_files`, `upsert_capture_output_gain_db`, `ir_capture_gain_db`, `audit_ir_plugin`, `strip_top_level_output_gain_db` — names used consistently across tasks. `AuditReport` reused with NaN for the IR path's unused LUFS/peak fields (mean gain reported in the existing columns).
