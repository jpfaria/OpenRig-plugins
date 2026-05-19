# Cab/Body IR Loudness Compensation — Design

- **Issue:** jpfaria/OpenRig-plugins#8
- **Cross-repo dependency:** jpfaria/OpenRig#491 (engine reads `output_gain_db`)
- **Date:** 2026-05-18
- **Status:** approved (design), pending spec review

## Problem

`loudness_audit` levels loudness only for `amp | preamp | gain_pedal` NAM
captures. `cab` and `body` plugins are IR-backed (`backend: ir`, `.wav`) and are
currently excluded under the (wrong) assumption that an IR carries no loudness
signature. An IR is a linear filter with real insertion loss (commonly
-12 to -18 dB for speaker cabs). Uncompensated, inserting a cab/body block
audibly drops the chain level — the chain loses volume passing through the block.

## Goal

A cab/body block must be **loudness-transparent**: passing the signal through
the IR must not lose volume. Same boost-only philosophy already applied to
amps. Each IR is measured individually and the manifest records how much to
compensate.

## Scope

- 20 `type: cab` plugins, 114 `type: body` plugins (134 manifests) in
  `plugins/source/`, `backend: ir`, `.wav` impulse responses.
- Each manifest holds a **grid of `captures:`** (cab: mic/speaker/position;
  body: flavor). Each capture is a distinct `.wav` with its own insertion loss.
- Out of scope: native LV2 plugins (no capture to measure); `eq`/other
  spectral/time FX; the engine-side read (tracked in OpenRig#491, coordinated
  here).

## Design

### Per-capture compensation (canonical field location)

A single per-plugin number cannot make every capture in the grid
loudness-neutral — different IRs in the same plugin have materially different
insertion loss. Therefore `output_gain_db` is written **inside each `captures:`
entry**.

This is the **single canonical location for `output_gain_db` for every block
type**, amps included. One field, one place, one producer, one consumer — no
second location, no producer/consumer drift. (That drift is exactly the failure
class of OpenRig#491; this design must not reintroduce it for cab/body.)

### Measurement (IR by IR)

1. Reuse the existing deterministic synthetic guitar DI
   (`default_guitar_di`, `tools/loudness_audit/src/synthetic_di.rs`) — the
   same ruler the amp path already uses. No new reference signal.
2. For each capture: load its `.wav` IR, convolve the DI through it.
3. Measure integrated LUFS of the DI input and of the convolved output.
4. `output_gain_db = max(0, LUFS_in − LUFS_out)` — the measured insertion loss.
   - **Boost-only**: never attenuate (a chain must never lose volume; it also
     must not be made louder than unity by this baseline).
   - **Peak guard**: same ceiling logic as the amp path
     (`PEAK_CEILING_DBFS`) so the makeup never forces clipping; if the boost
     needed for unity loudness would exceed the ceiling, the peak wins.
5. Write `output_gain_db` into that capture's manifest entry, preserving YAML
   ordering/spacing (same discipline as the existing `upsert_output_gain_db`).

### Tool changes — `tools/loudness_audit`

- `is_loudness_normalisable` accepts `cab` and `body`.
- New IR measurement path parallel to the existing NAM path: when the block is
  IR-backed, load `.wav` and convolve instead of `open_model_diag` /
  `nam_process`. Reuse the existing `loudness` (LUFS, peak) module unchanged.
- Writer emits `output_gain_db` per `captures:` entry. Amp path migrates to the
  same per-capture location for consistency (amps have a single capture, so the
  numeric result is unchanged — only the field moves into the capture entry).
- Unit and field name stay `output_gain_db` (dB), matching OpenRig#491's
  canonical decision. No `_pct`.

### Engine coupling (OpenRig#491 — blocking)

The engine must resolve `output_gain_db` **per selected capture**, not only a
manifest top-level value. If #491 ships top-level-only, cab/body compensation
does not work and amps-with-grids (future) would silently regress. The
per-capture contract must be agreed and reflected on the engine side before
either repo codes the change. A coordinating comment links #8 ↔ #491.

## Data flow

```
synthetic DI ──▶ convolve(.wav IR) ──▶ LUFS_out
      │                                   │
      └────────────▶ LUFS_in ─────────────┘
                          │
            output_gain_db = max(0, LUFS_in − LUFS_out), peak-clamped
                          │
                          ▼
        manifest.yaml  captures:[i].output_gain_db   (per capture)
                          │
                          ▼
        OpenRig engine: applies per selected capture (OpenRig#491)
```

## Testing / verification

- Unit: insertion-loss math (known synthetic IR → expected dB), boost-only
  clamp, peak-ceiling clamp, YAML upsert preserves structure for a capture grid.
- Catalog: run over all 134 cab/body manifests; assert every capture gets a
  finite `output_gain_db >= 0`; assert no manifest structural diff beyond the
  inserted field.
- End-to-end (with OpenRig#491): inserting a cab/body block in OpenRig is
  loudness-transparent (measured, not assumed).
- Gate: `cargo run --release --bin pack_plugins` exit 0 before any push.

## Docs in sync (same change — repo LAW)

- `tools/loudness_audit` module doc comment (now covers IR/cab/body).
- `.claude/skills/openrig-code-quality/SKILL.md` if methodology/anti-pattern
  changes.
- Any `docs/**` describing the loudness system.
- `grep -rn` for the old "only amp/preamp/gain_pedal" wording across `*.md`,
  `CLAUDE.md`, skills, and fix all in the same commit.

## Risks / open points

- **Engine coordination is the critical path.** Producer (this repo) and
  consumer (OpenRig#491) must agree on per-capture before coding. Tracked as a
  blocking acceptance criterion.
- IR sample rate / length normalization: the DI and IR must be matched in
  sample rate before convolution (`body` files are explicitly `*_48000.wav`);
  the tool must resample or assert consistency. To be settled in the
  implementation plan.
